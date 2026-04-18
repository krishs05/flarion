use std::io::{self, Write};

use crate::cli::client::FlarionClient;
use crate::cli::endpoint_file::{self, EndpointEntry};

pub async fn run(name: String) -> anyhow::Result<()> {
    let path = endpoint_file::default_path()
        .ok_or_else(|| anyhow::anyhow!("no client config path available"))?;
    let mut file = endpoint_file::load(&path).unwrap_or_default();

    print!("URL [http://127.0.0.1:8080]: ");
    io::stdout().flush()?;
    let mut url_in = String::new();
    io::stdin().read_line(&mut url_in)?;
    let url_in = url_in.trim();
    let url = if url_in.is_empty() {
        "http://127.0.0.1:8080".to_string()
    } else {
        url_in.to_string()
    };

    print!("API key (blank for none, or ${{ENV_VAR}} to reference an env var): ");
    io::stdout().flush()?;
    let mut key_in = String::new();
    io::stdin().read_line(&mut key_in)?;
    let key_in = key_in.trim();
    let api_key = if key_in.is_empty() {
        None
    } else {
        Some(key_in.to_string())
    };

    let entry = EndpointEntry {
        url: url.clone(),
        api_key,
        api_key_cmd: None,
    };

    // Verify connectivity — best effort.
    match entry.resolve(&name) {
        Ok(endpoint) => match FlarionClient::new(endpoint) {
            Ok(client) => match client.version().await {
                Ok(v) => println!("connected — server v{}", v.version),
                Err(e) => eprintln!("warning: couldn't verify endpoint ({e}). saving anyway."),
            },
            Err(e) => eprintln!("warning: couldn't construct client ({e}). saving anyway."),
        },
        Err(e) => eprintln!("warning: couldn't resolve endpoint ({e}). saving anyway."),
    }

    file.endpoints.insert(name.clone(), entry);
    if file.default.is_none() {
        file.default = Some(name);
    }
    endpoint_file::save(&path, &file)?;
    println!("saved to {}", path.display());
    Ok(())
}
