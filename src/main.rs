use clap::Parser;
use flarion::config::{BackendType, Cli, ConfigError, FlarionConfig, ModelConfig};
use flarion::engine::backend::InferenceBackend;
use flarion::engine::llama::LlamaBackend;
use flarion::engine::registry::BackendRegistry;
use flarion::{engine, metrics, routing, server};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

/// Refuse to start if the server would accept unauthenticated connections on
/// a publicly-routable interface. Loopback binds are allowed but emit a
/// warning so operators don't miss the open posture.
fn validate_security_posture(cfg: &FlarionConfig) -> Result<(), ConfigError> {
    if cfg.server.api_keys.is_empty()
        && !cfg.server.allow_unauthenticated
        && !cfg.server.binds_loopback()
    {
        return Err(ConfigError::PublicBindRequiresAuth {
            host: cfg.server.host.clone(),
        });
    }
    if cfg.server.api_keys.is_empty() && cfg.server.binds_loopback() {
        tracing::warn!(
            host = %cfg.server.host,
            "server running UNAUTHENTICATED on loopback — do not expose this interface \
             to a public network without setting [server].api_keys"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = FlarionConfig::load(&cli.config)?;
    config.apply_cli_overrides(&cli);

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    tracing::info!("flarion v{} starting up", env!("CARGO_PKG_VERSION"));

    if let Err(e) = config.validate() {
        tracing::error!("invalid config: {e}");
        std::process::exit(1);
    }

    if let Err(e) = validate_security_posture(&config) {
        tracing::error!("security posture check failed: {e}");
        std::process::exit(1);
    }

    let metrics_handle = if config.metrics.enabled {
        match metrics::install() {
            Ok(h) => {
                tracing::info!(path = %config.metrics.path, "metrics exporter installed");
                Some(h)
            }
            Err(e) => {
                tracing::error!("failed to install metrics exporter: {e}");
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let resident_set = flarion::engine::scheduling::ResidentSet::new(config.server.vram_budget_mb);
    flarion::metrics::set_vram_budget(config.server.vram_budget_mb);

    let mut registry = BackendRegistry::new();
    for model_cfg in &config.models {
        match load_backend(model_cfg, resident_set.clone()).await {
            Ok(backend) => {
                tracing::info!(model_id = %model_cfg.id, "model loaded successfully");
                registry.insert(model_cfg.id.clone(), backend);
            }
            Err(e) => {
                tracing::error!(
                    model_id = %model_cfg.id,
                    error = %e,
                    "failed to load model; aborting startup"
                );
                std::process::exit(1);
            }
        }
    }

    // `validate()` already checked id collisions and target resolvability,
    // so compile errors below are unexpected and abort startup.
    for route_cfg in &config.routes {
        let default_timeout = route_cfg
            .first_token_timeout_ms
            .map(std::time::Duration::from_millis)
            .unwrap_or(std::time::Duration::from_secs(60));
        let compiled = match routing::rules::compile_route(route_cfg, default_timeout, |id| {
            registry.get(id)
        }) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(route = %route_cfg.id, "failed to compile route: {e}");
                std::process::exit(1);
            }
        };
        let routed = routing::routed_backend::RoutedBackend::new(compiled, 0);
        registry.insert(route_cfg.id.clone(), Arc::new(routed));
        tracing::info!(route_id = %route_cfg.id, "route registered");
    }

    let registry = Arc::new(registry);
    tracing::info!(loaded = registry.len(), "all models and routes loaded");

    let app = server::create_router(
        registry.clone(),
        &config.server,
        &config.metrics,
        metrics_handle.clone(),
    );

    let main_addr = format!("{}:{}", config.server.host, config.server.port);
    let main_listener = tokio::net::TcpListener::bind(&main_addr).await?;
    tracing::info!("listening on {main_addr}");

    // When `[metrics].bind` is set we serve `/metrics` ONLY on this
    // dedicated listener and skip mounting it on the main router. No auth
    // layer is attached — operators are expected to bind to a trusted
    // interface (typically loopback) so scrapers don't need credentials.
    let metrics_listener = if config.metrics.enabled
        && let Some(ref bind_addr) = config.metrics.bind
    {
        let handle = metrics_handle
            .clone()
            .expect("metrics enabled ⇒ handle installed");
        let metrics_app: axum::Router = axum::Router::new().route(
            config.metrics.path.as_str(),
            axum::routing::get(metrics::metrics_handler).with_state(handle),
        );
        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        tracing::info!("metrics listener on {bind_addr}");
        Some((listener, metrics_app))
    } else {
        None
    };

    let grace = std::time::Duration::from_secs(config.server.shutdown_grace_secs);

    let serve_result: anyhow::Result<()> = match metrics_listener {
        Some((m_listener, m_app)) => {
            let main_fut =
                axum::serve(main_listener, app).with_graceful_shutdown(shutdown_signal());
            let metrics_fut =
                axum::serve(m_listener, m_app).with_graceful_shutdown(shutdown_signal());

            let main_timed = tokio::time::timeout(grace, main_fut);
            let metrics_timed = tokio::time::timeout(grace, metrics_fut);

            let (main_res, metrics_res) = tokio::join!(main_timed, metrics_timed);
            match (main_res, metrics_res) {
                (Ok(Ok(())), Ok(Ok(()))) => Ok(()),
                (Ok(Err(e)), _) | (_, Ok(Err(e))) => Err(e.into()),
                (Err(_), _) | (_, Err(_)) => {
                    tracing::warn!(
                        grace_secs = grace.as_secs(),
                        "shutdown grace exceeded; forcing shutdown"
                    );
                    Ok(())
                }
            }
        }
        None => {
            let main_fut =
                axum::serve(main_listener, app).with_graceful_shutdown(shutdown_signal());
            match tokio::time::timeout(grace, main_fut).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e.into()),
                Err(_) => {
                    tracing::warn!(
                        grace_secs = grace.as_secs(),
                        "shutdown grace exceeded; forcing shutdown"
                    );
                    Ok(())
                }
            }
        }
    };

    tracing::info!("draining backend workers");
    for backend in registry.backends() {
        backend.shutdown(std::time::Duration::from_secs(2)).await;
    }

    serve_result?;
    tracing::info!("server shut down gracefully");
    Ok(())
}

async fn load_backend(
    cfg: &ModelConfig,
    resident_set: std::sync::Arc<flarion::engine::scheduling::ResidentSet>,
) -> anyhow::Result<Arc<dyn InferenceBackend>> {
    match cfg.backend {
        BackendType::Local => {
            let path = cfg
                .path
                .as_ref()
                .expect("local backend path must be set — earlier validation ensures this");
            let estimated_mb = flarion::engine::scheduling::estimate_vram_mb(path, cfg.vram_mb)
                .map_err(|e| anyhow::anyhow!("VRAM estimation failed for '{}': {e}", cfg.id))?;
            let backend = LlamaBackend::new(cfg, resident_set, estimated_mb)?;
            if !cfg.lazy {
                backend.load().await?;
            } else {
                tracing::info!(
                    model_id = %cfg.id,
                    estimated_mb,
                    "model registered as lazy; will load on first request"
                );
            }
            Ok(Arc::new(backend))
        }
        BackendType::Openai => {
            let backend = engine::openai::OpenAICompatibleBackend::new(cfg, "openai")?;
            backend.load().await?;
            Ok(Arc::new(backend))
        }
        BackendType::Groq => {
            let backend = engine::openai::OpenAICompatibleBackend::new(cfg, "groq")?;
            backend.load().await?;
            Ok(Arc::new(backend))
        }
        BackendType::Anthropic => {
            let backend = engine::anthropic::AnthropicBackend::new(cfg)?;
            backend.load().await?;
            Ok(Arc::new(backend))
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flarion::config::{BackendType, ModelConfig, ServerConfig};
    use std::path::PathBuf;

    fn dummy_config(server: ServerConfig) -> FlarionConfig {
        FlarionConfig {
            server,
            models: vec![ModelConfig {
                id: "m".into(),
                backend: BackendType::Local,
                path: Some(PathBuf::from("/tmp/m.gguf")),
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: false,
                vram_mb: None,
            }],
            ..FlarionConfig::default()
        }
    }

    #[test]
    fn posture_accepts_public_with_keys() {
        let cfg = dummy_config(ServerConfig {
            host: "0.0.0.0".into(),
            api_keys: vec!["k".into()],
            ..ServerConfig::default()
        });
        assert!(validate_security_posture(&cfg).is_ok());
    }

    #[test]
    fn posture_accepts_loopback_without_keys() {
        let cfg = dummy_config(ServerConfig {
            host: "127.0.0.1".into(),
            ..ServerConfig::default()
        });
        assert!(validate_security_posture(&cfg).is_ok());
    }

    #[test]
    fn posture_rejects_public_without_keys() {
        let cfg = dummy_config(ServerConfig {
            host: "0.0.0.0".into(),
            ..ServerConfig::default()
        });
        let err = validate_security_posture(&cfg).unwrap_err();
        assert!(matches!(err, ConfigError::PublicBindRequiresAuth { .. }));
    }

    #[test]
    fn posture_accepts_public_with_opt_in() {
        let cfg = dummy_config(ServerConfig {
            host: "0.0.0.0".into(),
            allow_unauthenticated: true,
            ..ServerConfig::default()
        });
        assert!(validate_security_posture(&cfg).is_ok());
    }

    #[tokio::test]
    async fn mock_backend_shutdown_is_noop_and_fast() {
        use flarion::engine::backend::InferenceBackend;
        use flarion::engine::testing::MockBackend;
        use std::time::Duration;

        let backend: Arc<dyn InferenceBackend> = Arc::new(MockBackend::succeeding("m", "ok"));
        let start = std::time::Instant::now();
        backend.shutdown(Duration::from_secs(30)).await;
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
