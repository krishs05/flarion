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

impl crate::cli::resolve::EndpointArgs for HealthArgs {
    fn url(&self) -> Option<&str> { self.url.as_deref() }
    fn api_key(&self) -> Option<&str> { self.api_key.as_deref() }
    fn endpoint(&self) -> Option<&str> { self.endpoint.as_deref() }
    fn client_config(&self) -> Option<&std::path::Path> { self.client_config.as_deref() }
}

pub async fn run(args: HealthArgs) -> anyhow::Result<()> {
    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
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

