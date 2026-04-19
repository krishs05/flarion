//! Interactive chat REPL with arrow-key history, slash commands,
//! streaming responses, and mid-stream Ctrl-C cancel.

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

pub async fn run(args: super::ChatArgs) -> anyhow::Result<()> {
    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;

    let mut model = match args.model.as_deref() {
        Some(m) => m.to_string(),
        None => {
            eprintln!("flarion: --model <id> is required for REPL (can switch later with /model)");
            std::process::exit(2);
        }
    };

    let history_path = history_file();
    let mut rl = DefaultEditor::new()?;
    if let Some(p) = &history_path {
        let _ = rl.load_history(p);
    }

    let mut messages: Vec<crate::api::types::ChatMessage> = Vec::new();
    if let Some(sys) = &args.system {
        messages.push(crate::api::types::ChatMessage {
            role: "system".into(),
            content: sys.clone(),
        });
    }

    println!("▲ flarion chat · model={model}  (/exit /clear /model <id> /help)");

    loop {
        let prompt = format!("{model}> ");
        let line = match rl.readline(&prompt) {
            Ok(l) => l,
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(e) => { eprintln!("read error: {e}"); break; }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        let _ = rl.add_history_entry(trimmed);

        if let Some(rest) = trimmed.strip_prefix('/') {
            let mut parts = rest.split_whitespace();
            match parts.next() {
                Some("exit") | Some("quit") | Some("q") => break,
                Some("help") | Some("h") | Some("?") => {
                    println!("  /exit            quit");
                    println!("  /clear           reset conversation history");
                    println!("  /model <id>      switch model");
                }
                Some("clear") => {
                    messages.clear();
                    if let Some(sys) = &args.system {
                        messages.push(crate::api::types::ChatMessage {
                            role: "system".into(),
                            content: sys.clone(),
                        });
                    }
                    println!("(history cleared)");
                }
                Some("model") => {
                    if let Some(new_id) = parts.next() {
                        model = new_id.to_string();
                        println!("(model → {model})");
                    } else {
                        println!("usage: /model <id>");
                    }
                }
                Some(other) => { println!("unknown command: /{other} (try /help)"); }
                None => {}
            }
            continue;
        }

        // Append user turn
        messages.push(crate::api::types::ChatMessage {
            role: "user".into(),
            content: trimmed.to_string(),
        });

        let req = crate::api::types::ChatCompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            stream: true,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: args.max_tokens,
            stop: vec![],
            seed: None,
        };

        let collected = send_and_stream(&client, req).await;
        if !collected.is_empty() {
            messages.push(crate::api::types::ChatMessage {
                role: "assistant".into(),
                content: collected,
            });
        }
    }

    if let Some(p) = &history_path {
        // Ensure parent directory exists before saving.
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = rl.save_history(p);
    }
    Ok(())
}

async fn send_and_stream(
    client: &crate::cli::client::FlarionClient,
    req: crate::api::types::ChatCompletionRequest,
) -> String {
    use futures_util::StreamExt;
    use std::io::Write;

    let mut stream = match client.chat_stream(req).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("\nchat error: {e}");
            return String::new();
        }
    };
    let mut stdout = std::io::stdout();
    let mut collected = String::new();
    let mut cancel = Box::pin(tokio::signal::ctrl_c());

    loop {
        tokio::select! {
            _ = &mut cancel => {
                println!("\n[canceled]");
                break;
            }
            maybe = stream.next() => match maybe {
                Some(Ok(chunk)) => {
                    for choice in &chunk.choices {
                        if let Some(ref content) = choice.delta.content {
                            let _ = stdout.write_all(content.as_bytes());
                            let _ = stdout.flush();
                            collected.push_str(content);
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
    collected
}

fn history_file() -> Option<std::path::PathBuf> {
    dirs::data_local_dir().map(|d| d.join("flarion").join("chat_history"))
}
