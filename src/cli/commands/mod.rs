use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub mod endpoints;
pub mod gpu;
pub mod health;
pub mod login;
pub mod models;
pub mod requests;
pub mod routes;
pub mod serve;
pub mod status;
pub mod version;

#[derive(Parser, Debug)]
#[command(
    name = "flarion",
    version,
    about = "A Rust-native LLM inference gateway"
)]
pub struct FlarionCli {
    #[command(subcommand)]
    pub command: Option<Command>,

    // Compat-shim flags: `flarion -c file.toml` should still work.
    // ROOT-LEVEL ONLY — no `global = true` to avoid collisions with the
    // matching flags on the `serve` subcommand.
    #[arg(short = 'c', long = "config", hide = true)]
    pub legacy_config: Option<PathBuf>,
    #[arg(long = "host", hide = true)]
    pub legacy_host: Option<String>,
    #[arg(short = 'p', long = "port", hide = true)]
    pub legacy_port: Option<u16>,
    #[arg(long = "log-level", hide = true)]
    pub legacy_log_level: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the Flarion inference gateway server.
    Serve(crate::config::Cli),
    /// Show GPU status.
    Gpu(crate::cli::commands::gpu::GpuArgs),
    /// Liveness check — hits /health.
    Health(crate::cli::commands::health::HealthArgs),
    /// Inspect a running Flarion server.
    Status(crate::cli::commands::status::StatusArgs),
    /// Manage models (list, show, load, unload, pin, unpin).
    Models(crate::cli::commands::models::ModelsArgs),
    /// Tail the server's request log (optionally --follow via SSE).
    Requests(crate::cli::commands::requests::RequestsArgs),
    /// Inspect configured routes.
    Routes(crate::cli::commands::routes::RoutesArgs),
    /// Manage named endpoints in the client config.
    Endpoints(crate::cli::commands::endpoints::EndpointsArgs),
    /// Interactive first-run wizard to add an endpoint.
    Login { name: String },
    /// Print version info for the client + any reachable server.
    Version(crate::cli::commands::version::VersionArgs),
}

pub async fn dispatch() -> anyhow::Result<()> {
    let parsed = FlarionCli::parse();
    match parsed.command {
        Some(Command::Serve(args)) => serve::run(args).await,
        Some(Command::Gpu(args)) => gpu::run(args).await,
        Some(Command::Health(args)) => health::run(args).await,
        Some(Command::Status(args)) => status::run(args).await,
        Some(Command::Models(args)) => models::run(args).await,
        Some(Command::Requests(args)) => requests::run(args).await,
        Some(Command::Routes(args)) => routes::run(args).await,
        Some(Command::Endpoints(args)) => endpoints::run(args).await,
        Some(Command::Login { name }) => login::run(name).await,
        Some(Command::Version(args)) => version::run(args).await,
        None => {
            // Compat: `flarion -c foo.toml` with no subcommand → act like serve.
            if parsed.legacy_config.is_some() {
                eprintln!(
                    "warning: 'flarion -c <file>' without a subcommand is deprecated; use 'flarion serve -c <file>'"
                );
                let cli = crate::config::Cli {
                    config: parsed
                        .legacy_config
                        .unwrap_or_else(|| PathBuf::from("flarion.toml")),
                    host: parsed.legacy_host,
                    port: parsed.legacy_port,
                    log_level: parsed.legacy_log_level,
                };
                return serve::run(cli).await;
            }
            // No subcommand, no flags: TUI placeholder.
            eprintln!(
                "flarion: no subcommand given. TUI is not yet available — run 'flarion --help' to see subcommands."
            );
            std::process::exit(2);
        }
    }
}
