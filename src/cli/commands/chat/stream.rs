//! Streaming one-shot chat: fire a chat completion with `stream = true`,
//! print tokens as they arrive.

use futures_util::StreamExt;
use std::io::Write;

pub async fn run_streaming_oneshot(
    client: &crate::cli::client::FlarionClient,
    req: crate::api::types::ChatCompletionRequest,
) -> anyhow::Result<()> {
    let mut stream = match client.chat_stream(req).await {
        Ok(s) => s,
        Err(crate::cli::error::ClientError::Unreachable { url, .. }) => {
            eprintln!("flarion: can't reach {url}"); std::process::exit(3);
        }
        Err(crate::cli::error::ClientError::Unauthorized) => {
            eprintln!("flarion: unauthorized"); std::process::exit(2);
        }
        Err(crate::cli::error::ClientError::NotFound { .. }) => {
            eprintln!("flarion: model not found"); std::process::exit(4);
        }
        Err(e) => { eprintln!("flarion: {e}"); std::process::exit(1); }
    };

    let mut stdout = std::io::stdout();
    let mut cancel = Box::pin(tokio::signal::ctrl_c());

    loop {
        tokio::select! {
            _ = &mut cancel => {
                println!("\n[canceled]");
                return Ok(());
            }
            maybe = stream.next() => match maybe {
                Some(Ok(chunk)) => {
                    for choice in &chunk.choices {
                        if let Some(ref content) = choice.delta.content {
                            stdout.write_all(content.as_bytes())?;
                            stdout.flush()?;
                        }
                    }
                }
                Some(Err(e)) => {
                    eprintln!("\n[stream error: {e}]");
                    break;
                }
                None => break,
            }
        }
    }
    println!();
    Ok(())
}
