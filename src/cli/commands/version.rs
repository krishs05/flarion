use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct VersionArgs {
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

pub async fn run(args: VersionArgs) -> anyhow::Result<()> {
    let client_version = env!("CARGO_PKG_VERSION");
    let features = {
        let mut v = Vec::new();
        if cfg!(feature = "cuda") {
            v.push("cuda");
        }
        if cfg!(feature = "hf_cuda") {
            v.push("hf_cuda");
        }
        v
    };

    let server_info = try_fetch_server(&args).await.ok();

    if args.json {
        let mut obj = serde_json::json!({
            "client": { "version": client_version, "features": features },
        });
        if let Some(s) = server_info {
            obj["server"] = serde_json::json!({
                "version": s.version,
                "endpoint": s.endpoint_name,
                "url": s.endpoint_url,
                "features": s.features,
            });
        }
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    let mode = crate::cli::branding::detect_mode();
    let mark = crate::cli::branding::render_mark(16, mode);
    print!("{mark}");
    println!("▲ FLARION {client_version}");
    println!(
        "  client  {}  ({})",
        client_version,
        if features.is_empty() {
            "—".to_string()
        } else {
            features.join(", ")
        }
    );
    match server_info {
        Some(s) => println!(
            "  server  {}  @ {} ({})  ·  features: {}",
            s.version,
            s.endpoint_name,
            s.endpoint_url,
            if s.features.is_empty() {
                "—".to_string()
            } else {
                s.features.join(", ")
            }
        ),
        None => println!("  server  unreachable (no running server found)"),
    }
    Ok(())
}

impl crate::cli::resolve::EndpointArgs for VersionArgs {
    fn url(&self) -> Option<&str> { self.url.as_deref() }
    fn api_key(&self) -> Option<&str> { self.api_key.as_deref() }
    fn endpoint(&self) -> Option<&str> { self.endpoint.as_deref() }
    fn client_config(&self) -> Option<&std::path::Path> { self.client_config.as_deref() }
}

struct ServerInfo {
    version: String,
    endpoint_name: String,
    endpoint_url: String,
    features: Vec<String>,
}

async fn try_fetch_server(args: &VersionArgs) -> anyhow::Result<ServerInfo> {
    use crate::cli::client::FlarionClient;

    let endpoint = crate::cli::resolve::resolve_from_args(args)?;
    let client = FlarionClient::new(endpoint.clone())?;
    let v = client.version().await?;
    Ok(ServerInfo {
        version: v.version,
        endpoint_name: endpoint.name,
        endpoint_url: endpoint.url,
        features: v.features,
    })
}
