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

struct ServerInfo {
    version: String,
    endpoint_name: String,
    endpoint_url: String,
    features: Vec<String>,
}

async fn try_fetch_server(args: &VersionArgs) -> anyhow::Result<ServerInfo> {
    use crate::cli::{
        client::FlarionClient,
        endpoint_file,
        resolve::{resolve, ResolveArgs},
    };

    let file = if let Some(p) = &args.client_config {
        Some(endpoint_file::load(p)?)
    } else if let Some(p) = endpoint_file::default_path() {
        endpoint_file::load(&p).ok()
    } else {
        None
    };

    let endpoint = resolve(
        &ResolveArgs {
            url_flag: args.url.clone(),
            api_key_flag: args.api_key.clone(),
            endpoint_name: args.endpoint.clone(),
        },
        file.as_ref(),
    )?;

    let client = FlarionClient::new(endpoint.clone())?;
    let v = client.version().await?;
    Ok(ServerInfo {
        version: v.version,
        endpoint_name: endpoint.name,
        endpoint_url: endpoint.url,
        features: v.features,
    })
}
