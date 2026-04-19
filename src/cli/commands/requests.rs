use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct RequestsArgs {
    #[command(subcommand)]
    pub command: RequestsCmd,
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
pub enum RequestsCmd {
    /// Print the last N requests; optionally follow new ones via SSE.
    Tail {
        #[arg(short = 'n', long, default_value = "50")]
        n: usize,
        #[arg(long)]
        follow: bool,
    },
}

impl crate::cli::resolve::EndpointArgs for RequestsArgs {
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

pub async fn run(args: RequestsArgs) -> anyhow::Result<()> {
    use futures_util::StreamExt;

    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;

    match args.command {
        RequestsCmd::Tail { n, follow } => {
            let history = match client.requests(n).await {
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
            for ev in &history {
                print_event(ev, args.json)?;
            }

            if follow {
                let mut stream = match client.stream_requests().await {
                    Ok(s) => s,
                    Err(crate::cli::error::ClientError::Unreachable { url, .. }) => {
                        eprintln!("flarion: stream can't reach {url}");
                        std::process::exit(3);
                    }
                    Err(e) => {
                        eprintln!("flarion: stream error: {e}");
                        std::process::exit(1);
                    }
                };
                while let Some(r) = stream.next().await {
                    match r {
                        Ok(ev) => print_event(&ev, args.json)?,
                        Err(e) => {
                            eprintln!("flarion: stream error: {e}");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_event(ev: &crate::admin::types::RequestEvent, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string(ev)?);
    } else {
        match ev {
            crate::admin::types::RequestEvent::Started {
                id,
                ts,
                backend,
                ..
            } => {
                println!("{ts}  started    {:<32}  backend={backend}", id);
            }
            crate::admin::types::RequestEvent::Completed {
                id,
                ts,
                backend,
                duration_ms,
                prompt_tokens,
                completion_tokens,
                ..
            } => {
                println!(
                    "{ts}  completed  {:<32}  backend={backend}  {duration_ms}ms  p/c={prompt_tokens}/{completion_tokens}",
                    id
                );
            }
            crate::admin::types::RequestEvent::Failed {
                id,
                ts,
                backend,
                reason,
                duration_ms,
            } => {
                println!(
                    "{ts}  FAILED     {:<32}  backend={backend}  {duration_ms}ms  {reason}",
                    id
                );
            }
            crate::admin::types::RequestEvent::Canceled {
                id,
                ts,
                backend,
                duration_ms,
            } => {
                println!(
                    "{ts}  canceled   {:<32}  backend={backend}  {duration_ms}ms",
                    id
                );
            }
            crate::admin::types::RequestEvent::FirstToken { id, ttft_ms, .. } => {
                println!(
                    "                            first-token {:<32}  ttft={ttft_ms}ms",
                    id
                );
            }
            crate::admin::types::RequestEvent::Gap { missed } => {
                println!("-- stream gap: {missed} events missed --");
            }
        }
    }
    Ok(())
}
