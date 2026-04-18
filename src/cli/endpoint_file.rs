use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::endpoint::Endpoint;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct EndpointFile {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub endpoints: BTreeMap<String, EndpointEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EndpointEntry {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_cmd: Option<String>,
}

/// Default path: `$XDG_CONFIG_HOME/flarion/config.toml` on Unix,
/// `%APPDATA%\flarion\config.toml` on Windows. Returns None if the system
/// config dir cannot be determined (very unusual — would be a broken env).
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("flarion").join("config.toml"))
}

pub fn load(path: &Path) -> std::io::Result<EndpointFile> {
    if !path.exists() {
        return Ok(EndpointFile::default());
    }
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn save(path: &Path, file: &EndpointFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl EndpointEntry {
    /// Resolve the entry into a concrete Endpoint. `api_key` literal is
    /// subjected to `${VAR}` interpolation. `api_key_cmd` shells out via
    /// `sh -c` (Unix) or `cmd /C` (Windows) to produce the key from stdout.
    pub fn resolve(&self, name: &str) -> std::io::Result<Endpoint> {
        let api_key = if let Some(literal) = &self.api_key {
            Some(interpolate_env(literal)?)
        } else if let Some(cmd) = &self.api_key_cmd {
            Some(run_key_cmd(cmd)?)
        } else {
            None
        };
        Ok(Endpoint {
            name: name.to_string(),
            url: self.url.clone(),
            api_key,
        })
    }
}

/// If `s` looks like `${VAR}`, resolve via `std::env::var`. Otherwise return as-is.
/// Returns an error only when the `${VAR}` form references an unset variable.
pub fn interpolate_env(s: &str) -> std::io::Result<String> {
    if s.starts_with("${") && s.ends_with('}') {
        let var = &s[2..s.len() - 1];
        std::env::var(var).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("env var {var} not set"),
            )
        })
    } else {
        Ok(s.to_string())
    }
}

fn run_key_cmd(cmd: &str) -> std::io::Result<String> {
    #[cfg(unix)]
    let out = std::process::Command::new("sh").arg("-c").arg(cmd).output()?;
    #[cfg(windows)]
    let out = std::process::Command::new("cmd")
        .args(["/C", cmd])
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "api_key_cmd failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_nonexistent_returns_empty_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.toml");
        let f = load(&path).unwrap();
        assert!(f.endpoints.is_empty());
        assert!(f.default.is_none());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut f = EndpointFile::default();
        f.default = Some("home".into());
        f.endpoints.insert("home".into(), EndpointEntry {
            url: "http://127.0.0.1:8080".into(),
            api_key: None,
            api_key_cmd: None,
        });
        save(tmp.path(), &f).unwrap();

        let loaded = load(tmp.path()).unwrap();
        assert_eq!(loaded.default, Some("home".into()));
        assert_eq!(loaded.endpoints.len(), 1);
        assert_eq!(loaded.endpoints.get("home").unwrap().url, "http://127.0.0.1:8080");
    }

    #[test]
    fn interpolate_env_returns_value_of_env_var() {
        // SAFETY: single-threaded test mutating process env; acceptable in unit tests.
        unsafe { std::env::set_var("FLARION_TEST_KEY_A", "abc123"); }
        assert_eq!(interpolate_env("${FLARION_TEST_KEY_A}").unwrap(), "abc123");
    }

    #[test]
    fn interpolate_env_passes_through_literal() {
        assert_eq!(interpolate_env("literal-string").unwrap(), "literal-string");
    }

    #[test]
    fn interpolate_env_errors_on_missing_var() {
        // Use a unique var name unlikely to collide
        let err = interpolate_env("${FLARION_TEST_MISSING_XYZ}").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn entry_resolve_with_literal_key() {
        let entry = EndpointEntry {
            url: "http://x".into(),
            api_key: Some("plain-key".into()),
            api_key_cmd: None,
        };
        let ep = entry.resolve("n").unwrap();
        assert_eq!(ep.name, "n");
        assert_eq!(ep.url, "http://x");
        assert_eq!(ep.api_key.as_deref(), Some("plain-key"));
    }

    #[test]
    fn entry_resolve_with_env_var_reference() {
        unsafe { std::env::set_var("FLARION_TEST_RESOLVE_KEY", "from-env"); }
        let entry = EndpointEntry {
            url: "http://x".into(),
            api_key: Some("${FLARION_TEST_RESOLVE_KEY}".into()),
            api_key_cmd: None,
        };
        let ep = entry.resolve("n").unwrap();
        assert_eq!(ep.api_key.as_deref(), Some("from-env"));
    }

    #[test]
    fn entry_resolve_with_no_key() {
        let entry = EndpointEntry {
            url: "http://x".into(),
            api_key: None,
            api_key_cmd: None,
        };
        let ep = entry.resolve("n").unwrap();
        assert!(ep.api_key.is_none());
    }

    #[test]
    fn default_path_is_not_empty() {
        // `dirs::config_dir()` returns None in unusual environments.
        // We can't assert a specific path across OSes, just that SOME path is returned.
        let p = default_path();
        assert!(p.is_some(), "expected default config path to be resolvable");
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_600_permissions_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        save(tmp.path(), &EndpointFile::default()).unwrap();
        let mode = std::fs::metadata(tmp.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
