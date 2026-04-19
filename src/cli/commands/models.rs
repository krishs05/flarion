use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCmd,
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
    #[arg(long, global = true)]
    pub yes: bool,
}

#[derive(Subcommand, Debug)]
pub enum ModelsCmd {
    /// List all configured models.
    List {
        #[arg(long)]
        loaded: bool,
        #[arg(long)]
        lazy: bool,
        #[arg(long)]
        pinned: bool,
    },
    /// Show detail for one model.
    Show { id: String },
    /// Force-load a lazy or unloaded model.
    Load { id: String },
    /// Unload a model (fails with 409 if busy).
    Unload { id: String },
    /// Pin a model to prevent eviction.
    Pin { id: String },
    /// Unpin a previously pinned model.
    Unpin { id: String },
}

impl crate::cli::resolve::EndpointArgs for ModelsArgs {
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

pub async fn run(args: ModelsArgs) -> anyhow::Result<()> {
    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;

    match &args.command {
        ModelsCmd::List { loaded, lazy, pinned } => {
            let mut models = fetch_or_exit(&client).await;
            if *loaded {
                models.retain(|m| m.state == "loaded");
            }
            if *lazy {
                models.retain(|m| m.lazy);
            }
            if *pinned {
                models.retain(|m| m.pinned);
            }
            if args.json {
                println!("{}", serde_json::to_string_pretty(&models)?);
            } else if models.is_empty() {
                println!("(no models match)");
            } else {
                for m in &models {
                    println!(
                        "{:<24}  {:<10} {:<10} pin={}  in-flight={}",
                        m.id,
                        m.backend,
                        m.state,
                        if m.pinned { "y" } else { "n" },
                        m.in_flight
                    );
                }
            }
        }
        ModelsCmd::Show { id } => {
            let m = fetch_or_exit(&client)
                .await
                .into_iter()
                .find(|m| m.id == *id)
                .unwrap_or_else(|| {
                    eprintln!("flarion: no such model: {id}");
                    std::process::exit(4);
                });
            if args.json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                println!("id:         {}", m.id);
                println!("backend:    {}", m.backend);
                println!("state:      {}", m.state);
                println!("pinned:     {}", m.pinned);
                println!("lazy:       {}", m.lazy);
                println!(
                    "vram_mb:    {}",
                    m.vram_mb
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "\u{2014}".into())
                );
                println!(
                    "gpus:       {}",
                    if m.gpus.is_empty() {
                        "\u{2014}".into()
                    } else {
                        m.gpus
                            .iter()
                            .map(|g| g.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    }
                );
                println!("in_flight:  {}", m.in_flight);
            }
        }
        ModelsCmd::Load { id } => {
            match client.load_model(id).await {
                Ok(()) => println!("loaded {id}"),
                Err(e) => handle_mutation_error(id, "load", e),
            }
        }
        ModelsCmd::Unload { id } => {
            if !args.yes
                && std::io::IsTerminal::is_terminal(&std::io::stdin())
                && !confirm(&format!("unload '{id}'?"))?
            {
                eprintln!("aborted");
                return Ok(());
            }
            match client.unload_model(id).await {
                Ok(()) => println!("unloaded {id}"),
                Err(e) => handle_mutation_error(id, "unload", e),
            }
        }
        ModelsCmd::Pin { id } => {
            match client.pin_model(id, true).await {
                Ok(()) => println!("pinned {id}"),
                Err(e) => handle_mutation_error(id, "pin", e),
            }
        }
        ModelsCmd::Unpin { id } => {
            if !args.yes
                && std::io::IsTerminal::is_terminal(&std::io::stdin())
                && !confirm(&format!("unpin '{id}'?"))?
            {
                eprintln!("aborted");
                return Ok(());
            }
            match client.pin_model(id, false).await {
                Ok(()) => println!("unpinned {id}"),
                Err(e) => handle_mutation_error(id, "unpin", e),
            }
        }
    }
    Ok(())
}

async fn fetch_or_exit(
    client: &crate::cli::client::FlarionClient,
) -> Vec<crate::admin::types::Model> {
    match client.models().await {
        Ok(v) => v,
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
    }
}

fn handle_mutation_error(id: &str, action: &str, e: crate::cli::error::ClientError) -> ! {
    match e {
        crate::cli::error::ClientError::NotFound { .. } => {
            eprintln!("flarion: no such model: {id}");
            std::process::exit(4);
        }
        crate::cli::error::ClientError::Conflict { reason } => {
            eprintln!("flarion: can't {action} '{id}': {reason}");
            std::process::exit(5);
        }
        crate::cli::error::ClientError::Unauthorized => {
            eprintln!("flarion: unauthorized");
            std::process::exit(2);
        }
        crate::cli::error::ClientError::Unreachable { url, .. } => {
            eprintln!("flarion: can't reach {url}");
            std::process::exit(3);
        }
        e => {
            eprintln!("flarion: {action} failed: {e}");
            std::process::exit(1);
        }
    }
}

fn confirm(prompt: &str) -> std::io::Result<bool> {
    use std::io::Write;
    print!("{prompt} [y/N] ");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(matches!(s.trim().to_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use crate::admin::types::Model;

    fn make_model(id: &str, state: &str, pinned: bool, lazy: bool) -> Model {
        Model {
            id: id.into(),
            backend: "local".into(),
            state: state.into(),
            pinned,
            lazy,
            vram_mb: None,
            gpus: vec![],
            in_flight: 0,
            last_used_s: None,
        }
    }

    #[test]
    fn filter_loaded_keeps_only_loaded() {
        let models = vec![
            make_model("a", "loaded", false, false),
            make_model("b", "unloaded", false, false),
            make_model("c", "loaded", false, false),
        ];
        let result: Vec<_> = models
            .into_iter()
            .filter(|m| m.state == "loaded")
            .collect();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|m| m.state == "loaded"));
    }

    #[test]
    fn filter_lazy_keeps_only_lazy() {
        let models = vec![
            make_model("a", "loaded", false, true),
            make_model("b", "loaded", false, false),
        ];
        let result: Vec<_> = models.into_iter().filter(|m| m.lazy).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "a");
    }

    #[test]
    fn filter_pinned_keeps_only_pinned() {
        let models = vec![
            make_model("a", "loaded", true, false),
            make_model("b", "loaded", false, false),
        ];
        let result: Vec<_> = models.into_iter().filter(|m| m.pinned).collect();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "a");
    }

    #[test]
    fn show_find_by_id_succeeds() {
        let models = [
            make_model("llama-3", "loaded", false, false),
            make_model("mistral", "unloaded", false, false),
        ];
        let found = models.iter().find(|m| m.id == "llama-3");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "llama-3");
    }

    #[test]
    fn show_find_by_id_missing_returns_none() {
        let models = [make_model("llama-3", "loaded", false, false)];
        let found = models.iter().find(|m| m.id == "does-not-exist");
        assert!(found.is_none());
    }

    #[test]
    fn vram_mb_formats_none_as_dash() {
        let m = make_model("x", "loaded", false, false);
        let display = m.vram_mb.map(|v| v.to_string()).unwrap_or_else(|| "\u{2014}".into());
        assert_eq!(display, "\u{2014}");
    }

    #[test]
    fn vram_mb_formats_some_as_number() {
        let mut m = make_model("x", "loaded", false, false);
        m.vram_mb = Some(4096);
        let display = m.vram_mb.map(|v| v.to_string()).unwrap_or_else(|| "\u{2014}".into());
        assert_eq!(display, "4096");
    }

    #[test]
    fn gpus_formats_empty_as_dash() {
        let m = make_model("x", "loaded", false, false);
        let display = if m.gpus.is_empty() {
            "\u{2014}".into()
        } else {
            m.gpus.iter().map(|g| g.to_string()).collect::<Vec<_>>().join(",")
        };
        assert_eq!(display, "\u{2014}");
    }

    #[test]
    fn gpus_formats_multiple_as_csv() {
        let mut m = make_model("x", "loaded", false, false);
        m.gpus = vec![0, 1, 3];
        let display = if m.gpus.is_empty() {
            "\u{2014}".into()
        } else {
            m.gpus.iter().map(|g| g.to_string()).collect::<Vec<_>>().join(",")
        };
        assert_eq!(display, "0,1,3");
    }
}
