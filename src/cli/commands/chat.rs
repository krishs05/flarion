use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct ChatArgs {
    /// Prompt text. Use "-" to read from stdin.
    pub prompt: Option<String>,

    /// Model id (must be configured in the server).
    #[arg(long)]
    pub model: Option<String>,

    /// Optional system message to prepend.
    #[arg(long)]
    pub system: Option<String>,

    /// Max tokens per response.
    #[arg(long, default_value = "512")]
    pub max_tokens: u32,

    /// Start an interactive REPL (implemented in Task 12).
    #[arg(long)]
    pub repl: bool,

    #[arg(long)] pub url: Option<String>,
    #[arg(long)] pub api_key: Option<String>,
    #[arg(long)] pub endpoint: Option<String>,
    #[arg(long)] pub client_config: Option<PathBuf>,
}

impl crate::cli::resolve::EndpointArgs for ChatArgs {
    fn url(&self) -> Option<&str> { self.url.as_deref() }
    fn api_key(&self) -> Option<&str> { self.api_key.as_deref() }
    fn endpoint(&self) -> Option<&str> { self.endpoint.as_deref() }
    fn client_config(&self) -> Option<&std::path::Path> { self.client_config.as_deref() }
}

pub async fn run(args: ChatArgs) -> anyhow::Result<()> {
    if args.repl {
        anyhow::bail!("--repl not yet implemented (lands in Task 12)");
    }

    let prompt = match args.prompt.as_deref() {
        Some("-") => {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        Some(p) => p.to_string(),
        None => {
            eprintln!("flarion: no prompt provided (positional arg or '-' for stdin required; use --repl for interactive mode once available)");
            std::process::exit(2);
        }
    };

    let model = match args.model.as_deref() {
        Some(m) => m.to_string(),
        None => {
            eprintln!("flarion: --model <id> is required");
            std::process::exit(2);
        }
    };

    let endpoint = crate::cli::resolve::resolve_from_args(&args)?;
    let client = crate::cli::client::FlarionClient::new(endpoint)?;

    let mut messages = Vec::new();
    if let Some(sys) = &args.system {
        messages.push(crate::api::types::ChatMessage {
            role: "system".into(),
            content: sys.clone(),
        });
    }
    messages.push(crate::api::types::ChatMessage {
        role: "user".into(),
        content: prompt,
    });

    let req = crate::api::types::ChatCompletionRequest {
        model,
        messages,
        stream: false,
        temperature: 0.7,
        top_p: 0.9,
        max_tokens: args.max_tokens,
        stop: vec![],
        seed: None,
    };

    let resp = match client.chat_nonstream(req).await {
        Ok(r) => r,
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

    for choice in &resp.choices {
        print!("{}", choice.message.content);
    }
    println!();
    Ok(())
}
