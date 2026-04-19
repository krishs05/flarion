use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCmd,
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
pub enum ConfigCmd {
    /// Show the server's effective config (redacted by default).
    Show {
        /// Show with secrets redacted (default).
        #[arg(long)]
        redacted: bool,
        /// Show the LOCAL config file unredacted (asks for confirmation).
        #[arg(long, conflicts_with = "redacted")]
        raw: bool,
        /// Path to the local config (only used with --raw).
        #[arg(short = 'c', long, default_value = "flarion.toml")]
        config: PathBuf,
    },
    /// Validate a local config file (no server round-trip).
    Validate {
        /// Path to the config file to validate.
        #[arg(short = 'c', long, default_value = "flarion.toml")]
        config: PathBuf,
    },
}

impl crate::cli::resolve::EndpointArgs for ConfigArgs {
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

pub async fn run(args: ConfigArgs) -> anyhow::Result<()> {
    match &args.command {
        ConfigCmd::Show { raw, config, .. } => {
            if *raw {
                if !args.yes
                    && std::io::IsTerminal::is_terminal(&std::io::stdin())
                    && !confirm(
                        "This prints the LOCAL config file unredacted \
                         (may include secret-like ${ENV} references). Continue?",
                    )?
                {
                    eprintln!("aborted");
                    return Ok(());
                }
                match std::fs::read_to_string(config) {
                    Ok(text) => {
                        if args.json {
                            // Parse then re-emit as JSON.
                            let cfg: crate::config::FlarionConfig =
                                toml::from_str(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
                            println!("{}", serde_json::to_string_pretty(&cfg)?);
                        } else {
                            print!("{text}");
                        }
                    }
                    Err(e) => {
                        eprintln!("flarion: can't read {}: {e}", config.display());
                        std::process::exit(1);
                    }
                }
            } else {
                // redacted (default) — fetch from server
                let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
                let client = crate::cli::client::FlarionClient::new(endpoint)?;
                let v = match client.effective_config().await {
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
                };
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    // Pretty-print as TOML if possible, otherwise fall back to JSON.
                    match toml::to_string_pretty(&v) {
                        Ok(t) => print!("{t}"),
                        Err(_) => println!("{}", serde_json::to_string_pretty(&v)?),
                    }
                }
            }
        }
        ConfigCmd::Validate { config } => {
            match crate::config::FlarionConfig::load(config) {
                Ok(_) => {
                    println!("✓ {} is valid", config.display());
                }
                Err(e) => {
                    eprintln!("✗ {}: {e}", config.display());
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
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
    use super::*;

    #[test]
    fn config_args_parse_show_default() {
        use clap::Parser;

        #[derive(Parser)]
        struct Cli {
            #[command(subcommand)]
            cmd: ConfigCmd,
        }

        let parsed = Cli::try_parse_from(["flarion", "show"]).unwrap();
        assert!(matches!(
            parsed.cmd,
            ConfigCmd::Show {
                raw: false,
                redacted: false,
                ..
            }
        ));
    }

    #[test]
    fn config_args_parse_show_raw() {
        use clap::Parser;

        #[derive(Parser)]
        struct Cli {
            #[command(subcommand)]
            cmd: ConfigCmd,
        }

        let parsed = Cli::try_parse_from(["flarion", "show", "--raw"]).unwrap();
        assert!(matches!(parsed.cmd, ConfigCmd::Show { raw: true, .. }));
    }

    #[test]
    fn config_args_parse_validate() {
        use clap::Parser;

        #[derive(Parser)]
        struct Cli {
            #[command(subcommand)]
            cmd: ConfigCmd,
        }

        let parsed =
            Cli::try_parse_from(["flarion", "validate", "-c", "my.toml"]).unwrap();
        match parsed.cmd {
            ConfigCmd::Validate { config } => {
                assert_eq!(config.to_str().unwrap(), "my.toml");
            }
            _ => panic!("expected Validate"),
        }
    }

    #[test]
    fn raw_and_redacted_conflict() {
        use clap::Parser;

        #[derive(Parser)]
        struct Cli {
            #[command(subcommand)]
            cmd: ConfigCmd,
        }

        let res = Cli::try_parse_from(["flarion", "show", "--raw", "--redacted"]);
        assert!(res.is_err(), "--raw and --redacted should conflict");
    }
}
