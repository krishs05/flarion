mod api;
mod config;
mod engine;
mod error;
mod server;

use clap::Parser;
use config::{Cli, FlarionConfig};
use engine::backend::InferenceBackend;
use engine::llama::LlamaBackend;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Parse CLI args
    let cli = Cli::parse();

    // 2. Load and parse config
    let mut config = FlarionConfig::load(&cli.config)?;

    // 3. Apply CLI overrides
    config.apply_cli_overrides(&cli);

    // 4. Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.logging.level)),
        )
        .init();

    tracing::info!(
        "flarion v{} starting up",
        env!("CARGO_PKG_VERSION")
    );

    // 5. Create and load backend
    let backend = LlamaBackend::new(&config.model)?;
    backend.load().await?;

    let backend: Arc<dyn engine::backend::InferenceBackend> = Arc::new(backend);

    tracing::info!(
        model_id = %config.model.id,
        "model loaded successfully"
    );

    // 6. Build router
    let app = server::create_router(backend);

    // 7. Bind and serve
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("server shut down gracefully");
    Ok(())
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
