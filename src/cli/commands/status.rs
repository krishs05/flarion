use clap::Args;
use std::path::PathBuf;

use crate::admin::types::Status;
use crate::cli::client::FlarionClient;
use crate::cli::endpoint_file;
use crate::cli::resolve::{ResolveArgs, resolve};

#[derive(Args, Debug)]
pub struct StatusArgs {
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

pub async fn run(args: StatusArgs) -> anyhow::Result<()> {
    let file = if let Some(p) = &args.client_config {
        Some(endpoint_file::load(p)?)
    } else if let Some(p) = endpoint_file::default_path() {
        endpoint_file::load(&p).ok()
    } else {
        None
    };

    let endpoint = resolve(&ResolveArgs {
        url_flag: args.url,
        api_key_flag: args.api_key,
        endpoint_name: args.endpoint,
    }, file.as_ref())?;

    let client = FlarionClient::new(endpoint.clone())?;
    let status = match client.status().await {
        Ok(s) => s,
        Err(crate::cli::error::ClientError::Unreachable { url, .. }) => {
            eprintln!("flarion: can't reach {url} — is the server running? (try 'flarion endpoints test')");
            std::process::exit(3);
        }
        Err(crate::cli::error::ClientError::Unauthorized) => {
            eprintln!("flarion: unauthorized — check api key for endpoint '{}'", endpoint.name);
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("flarion: {e}");
            std::process::exit(1);
        }
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print!("{}", render_human(&status));
    }
    Ok(())
}

fn render_human(s: &Status) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "flarion {} @ {}  (up {})\n",
        s.server.version,
        s.server.bind,
        format_uptime(s.server.uptime_s)
    ));
    out.push_str(&format!(
        "  features: {}\n",
        if s.server.features.is_empty() {
            "—".to_string()
        } else {
            s.server.features.join(", ")
        }
    ));
    out.push_str(&format!(
        "\n  in-flight: {}  ·  60s: {} req, {} err",
        s.in_flight_total,
        s.recent.requests_last_60s,
        s.recent.errors_last_60s,
    ));
    if let (Some(p50), Some(p95)) = (s.recent.ttft_p50_ms, s.recent.ttft_p95_ms) {
        out.push_str(&format!("  ·  TTFT p50 {p50}ms  p95 {p95}ms"));
    }
    out.push_str("\n\n  GPUs:\n");
    if s.gpus.is_empty() {
        out.push_str("    (none)\n");
    } else {
        for g in &s.gpus {
            out.push_str(&format!(
                "    [{}] {:<24}  {:>5} / {:>5} MB free\n",
                g.id, g.name, g.free_mb, g.budget_mb
            ));
        }
    }
    out.push_str("\n  Models:\n");
    if s.models.is_empty() {
        out.push_str("    (none configured)\n");
    } else {
        for m in &s.models {
            out.push_str(&format!(
                "    {:<24} {:<10} {:<10} in-flight={}\n",
                m.id, m.backend, m.state, m.in_flight
            ));
        }
    }
    out
}

fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::types::{RecentRollup, ServerInfo, Status};

    fn sample_status() -> Status {
        Status {
            server: ServerInfo {
                version: "0.9.0".into(),
                git_sha: None,
                uptime_s: 3720, // 1h 2m 0s
                bind: "127.0.0.1:8080".into(),
                features: vec!["cuda".into()],
            },
            gpus: vec![],
            models: vec![],
            in_flight_total: 0,
            recent: RecentRollup {
                requests_last_60s: 0,
                errors_last_60s: 0,
                ttft_p50_ms: None,
                ttft_p95_ms: None,
            },
        }
    }

    #[test]
    fn render_human_includes_version_and_uptime() {
        let out = render_human(&sample_status());
        assert!(out.contains("flarion 0.9.0"));
        assert!(out.contains("127.0.0.1:8080"));
        assert!(out.contains("1h 2m"));
        assert!(out.contains("cuda"));
    }

    #[test]
    fn render_human_shows_empty_gpus_placeholder() {
        let out = render_human(&sample_status());
        assert!(out.contains("(none)"));
    }

    #[test]
    fn format_uptime_handles_ranges() {
        assert_eq!(format_uptime(45), "45s");
        assert_eq!(format_uptime(125), "2m 5s");
        assert_eq!(format_uptime(3661), "1h 1m");
    }
}
