use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct GpuArgs {
    /// Specific GPU id to show (default: all).
    pub id: Option<u32>,
    #[arg(long)] pub url: Option<String>,
    #[arg(long)] pub api_key: Option<String>,
    #[arg(long)] pub endpoint: Option<String>,
    #[arg(long)] pub json: bool,
    #[arg(long)] pub client_config: Option<PathBuf>,
}

impl crate::cli::resolve::EndpointArgs for GpuArgs {
    fn url(&self) -> Option<&str> { self.url.as_deref() }
    fn api_key(&self) -> Option<&str> { self.api_key.as_deref() }
    fn endpoint(&self) -> Option<&str> { self.endpoint.as_deref() }
    fn client_config(&self) -> Option<&std::path::Path> { self.client_config.as_deref() }
}

pub async fn run(args: GpuArgs) -> anyhow::Result<()> {
    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;
    let gpus = match client.gpus().await {
        Ok(g) => g,
        Err(crate::cli::error::ClientError::Unreachable { url, .. }) => {
            eprintln!("flarion: can't reach {url}"); std::process::exit(3);
        }
        Err(crate::cli::error::ClientError::Unauthorized) => {
            eprintln!("flarion: unauthorized"); std::process::exit(2);
        }
        Err(e) => { eprintln!("flarion: {e}"); std::process::exit(1); }
    };

    let filtered: Vec<_> = match args.id {
        Some(id) => gpus.into_iter().filter(|g| g.id == id).collect(),
        None => gpus,
    };

    if args.id.is_some() && filtered.is_empty() {
        eprintln!("flarion: no such GPU (id={})", args.id.unwrap());
        std::process::exit(4);
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
    } else if filtered.is_empty() {
        println!("no GPUs detected (CPU-only build or no NVML-visible devices)");
    } else {
        for g in &filtered {
            let pct = if g.budget_mb > 0 {
                100 * (g.budget_mb - g.free_mb) / g.budget_mb
            } else { 0 };
            println!(
                "[{}] {:<28}  {:>5} / {:>5} MB free  ({}% used)  models: {}",
                g.id, g.name, g.free_mb, g.budget_mb, pct,
                if g.models.is_empty() { "—".into() } else { g.models.join(", ") },
            );
        }
    }
    Ok(())
}
