use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct HealthArgs {
    #[arg(long)]
    pub url: Option<String>,
    #[arg(long)]
    pub api_key: Option<String>,
    #[arg(long)]
    pub endpoint: Option<String>,
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub client_config: Option<PathBuf>,
}

pub async fn run(args: HealthArgs) -> anyhow::Result<()> {
    let endpoint = resolve_endpoint(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;
    let started = std::time::Instant::now();
    match client.health().await {
        Ok(v) => {
            let latency_ms = started.elapsed().as_millis();
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": v.get("status").cloned().unwrap_or(serde_json::json!(null)),
                        "latency_ms": latency_ms,
                    }))?
                );
            } else {
                println!(
                    "OK  status={}  ({}ms)",
                    v.get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?"),
                    latency_ms
                );
            }
            Ok(())
        }
        Err(crate::cli::error::ClientError::Unreachable { url, .. }) => {
            eprintln!("flarion: can't reach {url}");
            std::process::exit(3);
        }
        Err(e) => {
            eprintln!("flarion: {e}");
            std::process::exit(1);
        }
    }
}

fn resolve_endpoint(args: &HealthArgs) -> anyhow::Result<crate::cli::endpoint::Endpoint> {
    use crate::cli::{endpoint_file, resolve::{resolve, ResolveArgs}};

    let file = if let Some(p) = &args.client_config {
        Some(endpoint_file::load(p)?)
    } else if let Some(p) = endpoint_file::default_path() {
        endpoint_file::load(&p).ok()
    } else {
        None
    };

    Ok(resolve(
        &ResolveArgs {
            url_flag: args.url.clone(),
            api_key_flag: args.api_key.clone(),
            endpoint_name: args.endpoint.clone(),
        },
        file.as_ref(),
    )?)
}
