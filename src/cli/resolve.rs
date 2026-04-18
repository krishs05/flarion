use crate::cli::endpoint::Endpoint;
use crate::cli::endpoint_file::EndpointFile;
use crate::cli::error::ClientError;

#[derive(Debug, Default, Clone)]
pub struct ResolveArgs {
    pub url_flag: Option<String>,
    pub api_key_flag: Option<String>,
    pub endpoint_name: Option<String>,
}

pub fn resolve(args: &ResolveArgs, file: Option<&EndpointFile>) -> Result<Endpoint, ClientError> {
    // 1. Flags
    if let Some(url) = &args.url_flag {
        return Ok(Endpoint {
            name: "flag".into(),
            url: url.clone(),
            api_key: args.api_key_flag.clone(),
        });
    }
    // 2. Env vars
    if let Ok(url) = std::env::var("FLARION_URL") {
        let api_key = std::env::var("FLARION_API_KEY").ok();
        return Ok(Endpoint {
            name: "env".into(),
            url,
            api_key,
        });
    }
    // 3. Named endpoint from file
    if let (Some(name), Some(f)) = (&args.endpoint_name, file)
        && let Some(entry) = f.endpoints.get(name) {
            return entry.resolve(name).map_err(|e| ClientError::Server {
                status: 0,
                body: e.to_string(),
            });
        }
    // 4. Default endpoint from file
    if let Some(f) = file
        && let Some(name) = &f.default
        && let Some(entry) = f.endpoints.get(name) {
            return entry.resolve(name).map_err(|e| ClientError::Server {
                status: 0,
                body: e.to_string(),
            });
        }
    // 5. Local flarion.toml
    if let Ok(cfg) = crate::config::FlarionConfig::load(std::path::Path::new("flarion.toml")) {
        return Ok(Endpoint {
            name: "local".into(),
            url: format!("http://{}:{}", cfg.server.host, cfg.server.port),
            api_key: cfg.server.api_keys.first().cloned(),
        });
    }
    // 6. Loopback fallback
    Ok(Endpoint {
        name: "loopback".into(),
        url: "http://127.0.0.1:8080".into(),
        api_key: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::endpoint_file::{EndpointEntry, EndpointFile};

    #[test]
    fn flag_beats_everything_else() {
        let args = ResolveArgs {
            url_flag: Some("http://flag-wins".into()),
            api_key_flag: Some("flagkey".into()),
            endpoint_name: Some("home".into()),
        };
        let mut f = EndpointFile::default();
        f.endpoints.insert(
            "home".into(),
            EndpointEntry {
                url: "http://home".into(),
                api_key: None,
                api_key_cmd: None,
            },
        );
        let ep = resolve(&args, Some(&f)).unwrap();
        assert_eq!(ep.url, "http://flag-wins");
        assert_eq!(ep.api_key.as_deref(), Some("flagkey"));
    }

    #[test]
    fn named_endpoint_resolves_from_file() {
        let args = ResolveArgs {
            url_flag: None,
            api_key_flag: None,
            endpoint_name: Some("v100".into()),
        };
        let mut f = EndpointFile::default();
        f.endpoints.insert(
            "v100".into(),
            EndpointEntry {
                url: "http://v100.lan:8080".into(),
                api_key: None,
                api_key_cmd: None,
            },
        );
        let ep = resolve(&args, Some(&f)).unwrap();
        assert_eq!(ep.url, "http://v100.lan:8080");
        assert_eq!(ep.name, "v100");
    }

    #[test]
    fn default_endpoint_used_when_no_flags_or_name() {
        let mut f = EndpointFile {
            default: Some("home".into()),
            ..Default::default()
        };
        f.endpoints.insert(
            "home".into(),
            EndpointEntry {
                url: "http://home:8080".into(),
                api_key: None,
                api_key_cmd: None,
            },
        );
        let ep = resolve(&ResolveArgs::default(), Some(&f)).unwrap();
        assert_eq!(ep.url, "http://home:8080");
    }

    #[test]
    fn loopback_default_when_nothing_else() {
        // Make sure FLARION_URL isn't set from a prior test
        unsafe {
            std::env::remove_var("FLARION_URL");
        }
        // Run from a temp dir so there's no local flarion.toml
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let ep = resolve(&ResolveArgs::default(), None).unwrap();

        std::env::set_current_dir(prev).unwrap();

        assert_eq!(ep.url, "http://127.0.0.1:8080");
        assert_eq!(ep.name, "loopback");
    }
}
