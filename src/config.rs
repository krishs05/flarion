use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "flarion", version, about = "A Rust-native LLM inference gateway")]
pub struct Cli {
    /// Path to the configuration file
    #[arg(short, long, default_value = "flarion.toml")]
    pub config: PathBuf,

    /// Override server host
    #[arg(long)]
    pub host: Option<String>,

    /// Override server port
    #[arg(short, long)]
    pub port: Option<u16>,

    /// Override log level (trace, debug, info, warn, error)
    #[arg(long)]
    pub log_level: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FlarionConfig {
    pub server: ServerConfig,
    pub model: ModelConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub path: PathBuf,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: u32,
    pub threads: Option<u32>,
    pub batch_size: Option<u32>,
    pub seed: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
        }
    }
}

fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 8080 }
fn default_context_size() -> u32 { 4096 }
fn default_gpu_layers() -> u32 { 99 }
fn default_log_level() -> String { "info".to_string() }

impl FlarionConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::ReadFailed {
            path: path.to_path_buf(),
            source: e,
        })?;
        let config: FlarionConfig =
            toml::from_str(&content).map_err(|e| ConfigError::ParseFailed {
                path: path.to_path_buf(),
                source: e,
            })?;
        Ok(config)
    }

    pub fn apply_cli_overrides(&mut self, cli: &Cli) {
        if let Some(ref host) = cli.host {
            self.server.host = host.clone();
        }
        if let Some(port) = cli.port {
            self.server.port = port;
        }
        if let Some(ref level) = cli.log_level {
            self.logging.level = level.clone();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        source: toml::de::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[server]

[model]
id = "test-model"
path = "/tmp/model.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.model.id, "test-model");
        assert_eq!(config.model.context_size, 4096);
        assert_eq!(config.model.gpu_layers, 99);
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[server]
host = "127.0.0.1"
port = 3000

[model]
id = "qwen3-8b"
path = "/models/qwen3-8b.gguf"
context_size = 8192
gpu_layers = 40
threads = 8
batch_size = 512
seed = 42

[logging]
level = "debug"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.model.context_size, 8192);
        assert_eq!(config.model.gpu_layers, 40);
        assert_eq!(config.model.threads, Some(8));
        assert_eq!(config.model.batch_size, Some(512));
        assert_eq!(config.model.seed, Some(42));
        assert_eq!(config.logging.level, "debug");
    }

    #[test]
    fn test_cli_overrides() {
        let toml_str = r#"
[server]
host = "0.0.0.0"
port = 8080

[model]
id = "test"
path = "/tmp/model.gguf"
"#;
        let mut config: FlarionConfig = toml::from_str(toml_str).unwrap();
        let cli = Cli {
            config: PathBuf::from("flarion.toml"),
            host: Some("127.0.0.1".to_string()),
            port: Some(3000),
            log_level: Some("debug".to_string()),
        };
        config.apply_cli_overrides(&cli);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.logging.level, "debug");
    }

    #[test]
    fn test_invalid_toml_returns_error() {
        let result: Result<FlarionConfig, _> = toml::from_str("not valid toml [[[");
        assert!(result.is_err());
    }
}
