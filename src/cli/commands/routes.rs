use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct RoutesArgs {
    #[command(subcommand)]
    pub command: RoutesCmd,
    #[arg(long, global = true)]
    pub url: Option<String>,
    #[arg(long, global = true)]
    pub api_key: Option<String>,
    #[arg(long, global = true)]
    pub endpoint: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub client_config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum RoutesCmd {
    /// List configured routes.
    List,
    /// Show a single route's rules and hit counts.
    Show { id: String },
}

impl crate::cli::resolve::EndpointArgs for RoutesArgs {
    fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
    fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }
    fn endpoint(&self) -> Option<&str> {
        self.endpoint.as_deref()
    }
    fn client_config(&self) -> Option<&std::path::Path> {
        self.client_config.as_deref()
    }
}

pub async fn run(args: RoutesArgs) -> anyhow::Result<()> {
    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;
    let routes = match client.routes().await {
        Ok(r) => r,
        Err(crate::cli::error::ClientError::Unreachable { url, .. }) => {
            eprintln!("flarion: can't reach {url}");
            std::process::exit(3);
        }
        Err(crate::cli::error::ClientError::Unauthorized) => {
            eprintln!("flarion: unauthorized");
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("flarion: {e}");
            std::process::exit(1);
        }
    };

    match &args.command {
        RoutesCmd::List => {
            if args.json {
                println!("{}", serde_json::to_string_pretty(&routes)?);
            } else if routes.is_empty() {
                println!("(no routes configured)");
            } else {
                for r in &routes {
                    println!(
                        "{:<20}  {} rules  fallbacks={}",
                        r.id,
                        r.rules.len(),
                        r.fallback_count
                    );
                }
            }
        }
        RoutesCmd::Show { id } => {
            let r = routes
                .into_iter()
                .find(|r| r.id == *id)
                .unwrap_or_else(|| {
                    eprintln!("flarion: no such route: {id}");
                    std::process::exit(4);
                });
            if args.json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                println!("route:         {}", r.id);
                println!("fallbacks:     {}", r.fallback_count);
                println!("rules:");
                for rule in &r.rules {
                    println!(
                        "  {:<20}  hits={:<5}  targets=[{}]",
                        rule.name,
                        rule.hit_count,
                        rule.targets.join(", ")
                    );
                }
            }
        }
    }
    Ok(())
}
