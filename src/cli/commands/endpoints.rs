use clap::{Args, Subcommand};
use std::path::PathBuf;

use crate::cli::client::FlarionClient;
use crate::cli::endpoint_file::{self, EndpointEntry, EndpointFile};

#[derive(Args, Debug)]
pub struct EndpointsArgs {
    #[command(subcommand)]
    pub command: EndpointsCmd,

    #[arg(long, global = true)]
    pub client_config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum EndpointsCmd {
    /// List configured endpoints.
    List,
    /// Add or replace a named endpoint.
    Add {
        name: String,
        #[arg(long)]
        url: String,
        #[arg(long, conflicts_with_all = ["api_key_env", "api_key_cmd"])]
        api_key: Option<String>,
        #[arg(long, conflicts_with_all = ["api_key", "api_key_cmd"])]
        api_key_env: Option<String>,
        #[arg(long, conflicts_with_all = ["api_key", "api_key_env"])]
        api_key_cmd: Option<String>,
    },
    /// Remove an endpoint.
    Remove { name: String },
    /// Set the default endpoint.
    Use { name: String },
    /// Ping an endpoint and report version + latency.
    Test { name: Option<String> },
}

fn resolve_path(opt: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(p) = opt {
        return Ok(p);
    }
    endpoint_file::default_path()
        .ok_or_else(|| anyhow::anyhow!("no client config path available"))
}

pub async fn run(args: EndpointsArgs) -> anyhow::Result<()> {
    let path = resolve_path(args.client_config)?;
    let mut file = endpoint_file::load(&path)?;
    match args.command {
        EndpointsCmd::List => {
            list(&file, &path);
            Ok(())
        }
        EndpointsCmd::Add { name, url, api_key, api_key_env, api_key_cmd } => {
            let key = match (api_key, api_key_env) {
                (Some(k), _) => Some(k),
                (_, Some(var)) => Some(format!("${{{var}}}")),
                _ => None,
            };
            file.endpoints.insert(
                name.clone(),
                EndpointEntry { url, api_key: key, api_key_cmd },
            );
            if file.default.is_none() {
                file.default = Some(name);
            }
            endpoint_file::save(&path, &file)?;
            println!("saved to {}", path.display());
            Ok(())
        }
        EndpointsCmd::Remove { name } => {
            file.endpoints.remove(&name);
            if file.default.as_deref() == Some(name.as_str()) {
                file.default = file.endpoints.keys().next().cloned();
            }
            endpoint_file::save(&path, &file)?;
            println!("removed '{name}' from {}", path.display());
            Ok(())
        }
        EndpointsCmd::Use { name } => {
            if !file.endpoints.contains_key(&name) {
                anyhow::bail!("no such endpoint: {name}");
            }
            file.default = Some(name.clone());
            endpoint_file::save(&path, &file)?;
            println!("default endpoint set to '{name}'");
            Ok(())
        }
        EndpointsCmd::Test { name } => test_endpoint(&file, name.as_deref()).await,
    }
}

fn list(file: &EndpointFile, path: &std::path::Path) {
    println!("endpoints (from {})", path.display());
    if file.endpoints.is_empty() {
        println!("  (none — run 'flarion login <name>' or 'flarion endpoints add <name> --url <url>')");
        return;
    }
    for (name, entry) in &file.endpoints {
        let marker = if file.default.as_deref() == Some(name.as_str()) { "*" } else { " " };
        println!("  {marker} {:<12}  {}", name, entry.url);
    }
}

async fn test_endpoint(file: &EndpointFile, name: Option<&str>) -> anyhow::Result<()> {
    let name = name.or(file.default.as_deref()).ok_or_else(|| {
        anyhow::anyhow!("no endpoint specified and no default set")
    })?;
    let entry = file
        .endpoints
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("no such endpoint: {name}"))?;
    let endpoint = entry.resolve(name)?;
    let client = FlarionClient::new(endpoint)?;
    let start = std::time::Instant::now();
    match client.version().await {
        Ok(v) => {
            println!("{name}  OK  v{} ({} ms)", v.version, start.elapsed().as_millis());
            Ok(())
        }
        Err(e) => {
            eprintln!("{name}  FAIL  {e}");
            std::process::exit(3);
        }
    }
}
