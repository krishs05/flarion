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

    // Phase 2H: compute declared device count from [[models]].gpus.
    let declared_device_count = config
        .models
        .iter()
        .flat_map(|m| m.gpus.iter().copied())
        .max()
        .map(|m| m + 1)
        .unwrap_or(1);

    // Resolve per-device budgets (Auto mode ignores declared_device_count).
    let budgets = match config.server.resolve_vram_budgets(declared_device_count) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to resolve VRAM budgets: {e}");
            std::process::exit(1);
        }
    };

    let scheduler = flarion::engine::scheduling::Scheduler::new(budgets.clone());
    for (gpu_id, &budget_mb) in budgets.iter().enumerate() {
        flarion::metrics::set_vram_budget_on_gpu(gpu_id as u32, budget_mb);
    }

    let load_coordinator: Arc<tokio::sync::Mutex<()>> =
        Arc::new(tokio::sync::Mutex::new(()));

    let mut registry = BackendRegistry::new();
    for model_cfg in &config.models {
        match load_backend(model_cfg, scheduler.clone(), load_coordinator.clone()).await {
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

    // Bind the evictor onto every backend. Cloud backends treat this as a
    // no-op (their default trait impl). Local backends install a Weak so
    // the Registry→backend Arc cycle doesn't leak on shutdown.
    let evictor: Arc<dyn flarion::engine::backend::Evictor> = registry.clone();
    let evictor_weak = Arc::downgrade(&evictor);
    for backend in registry.backends() {
        backend.bind_evictor(evictor_weak.clone()).await;
    }

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

    // Do not wrap `axum::serve` in `tokio::time::timeout(shutdown_grace_secs)` —
    // that would stop the server after N seconds. `shutdown_grace_secs` is only
    // for clamping backend drain policy in config validation, not HTTP lifetime.

    let serve_result: anyhow::Result<()> = match metrics_listener {
        Some((m_listener, m_app)) => {
            let main_fut =
                axum::serve(main_listener, app).with_graceful_shutdown(shutdown_signal());
            let metrics_fut =
                axum::serve(m_listener, m_app).with_graceful_shutdown(shutdown_signal());

            let (main_res, metrics_res) = tokio::join!(main_fut, metrics_fut);
            match (main_res, metrics_res) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(e), _) | (_, Err(e)) => Err(e.into()),
            }
        }
        None => axum::serve(main_listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| e.into()),
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
    scheduler: std::sync::Arc<flarion::engine::scheduling::Scheduler>,
    load_coordinator: std::sync::Arc<tokio::sync::Mutex<()>>,
) -> anyhow::Result<Arc<dyn InferenceBackend>> {
    match cfg.backend {
        BackendType::Local => {
            let path = cfg
                .path
                .as_ref()
                .expect("local backend path must be set — earlier validation ensures this");
            let estimated_mb = flarion::engine::scheduling::estimate_vram_mb(path, cfg.vram_mb)
                .map_err(|e| anyhow::anyhow!("VRAM estimation failed for '{}': {e}", cfg.id))?;
            let backend = LlamaBackend::new(cfg, scheduler, estimated_mb, load_coordinator)?;
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
        BackendType::Hf => {
            #[cfg(feature = "hf_cuda")]
            {
                let backend = flarion::engine::hf::HfBackend::new(cfg)
                    .map_err(|e| anyhow::anyhow!("HF backend init failed for '{}': {e}", cfg.id))?;
                if !cfg.lazy {
                    backend.load().await?;
                } else {
                    tracing::info!(
                        model_id = %cfg.id,
                        "HF model registered as lazy; will load on first request"
                    );
                }
                Ok(Arc::new(backend))
            }
            #[cfg(not(feature = "hf_cuda"))]
            {
                Err(anyhow::anyhow!(
                    "model '{}': HF backend requested but binary was built without the `hf_cuda` feature",
                    cfg.id
                ))
            }
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
                pin: false,
                gpus: vec![],
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
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

    #[cfg(feature = "hf_cuda")]
    #[tokio::test]
    async fn test_load_backend_hf_lazy_registers_without_loading() {
        use flarion::engine::scheduling::Scheduler;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let cfg = ModelConfig {
            id: "hf-test".into(),
            backend: BackendType::Hf,
            path: None,
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
            lazy: true, // lazy so load() isn't called eagerly
            vram_mb: None,
            pin: false,
            gpus: Vec::new(),
            repo: Some("org/model".into()),
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        };
        let scheduler = Scheduler::new(vec![]);
        let coord = Arc::new(Mutex::new(()));
        let backend = load_backend(&cfg, scheduler, coord)
            .await
            .expect("lazy load_backend should succeed without invoking load");
        assert_eq!(backend.model_info().id, "hf-test");
        assert_eq!(backend.provider(), "hf");

        // Explicitly invoking load returns NotImplemented in Wave 1.
        let err = backend.load().await.unwrap_err();
        assert!(
            matches!(err, flarion::error::EngineError::NotImplemented(_)),
            "got: {err:?}"
        );
    }

    #[cfg(feature = "hf_cuda")]
    #[tokio::test]
    async fn test_load_backend_hf_eager_surfaces_not_implemented() {
        use flarion::engine::scheduling::Scheduler;
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let cfg = ModelConfig {
            id: "hf-eager".into(),
            backend: BackendType::Hf,
            path: None,
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
            lazy: false, // eager → load() called immediately
            vram_mb: None,
            pin: false,
            gpus: Vec::new(),
            repo: Some("org/model".into()),
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        };
        let scheduler = Scheduler::new(vec![]);
        let coord = Arc::new(Mutex::new(()));
        let result = load_backend(&cfg, scheduler, coord).await;
        assert!(result.is_err(), "expected eager HF load to fail with NotImplemented");
        let err = result.err().expect("already checked is_err");
        let msg = err.to_string();
        assert!(
            msg.contains("not implemented") || msg.contains("Wave 2"),
            "expected NotImplemented surface; got: {msg}"
        );
    }
}
