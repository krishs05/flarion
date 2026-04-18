use clap::Parser;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "flarion",
    version,
    about = "A Rust-native LLM inference gateway"
)]
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

#[derive(Debug, Deserialize, Clone, Default)]
pub struct FlarionConfig {
    pub server: ServerConfig,
    pub models: Vec<ModelConfig>,
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RouteConfig {
    pub id: String,
    pub first_token_timeout_ms: Option<u64>,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RuleConfig {
    pub name: String,
    #[serde(default)]
    pub matchers: Matchers,
    #[serde(default)]
    pub targets: Vec<String>,
    pub first_token_timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Matchers {
    pub stream: Option<bool>,
    pub prompt_tokens_gte: Option<u32>,
    pub prompt_tokens_lte: Option<u32>,
    pub message_count_gte: Option<u32>,
    pub message_count_lte: Option<u32>,
    pub has_system_prompt: Option<bool>,
    pub content_regex: Option<String>,
    #[serde(default)]
    pub header_equals: std::collections::HashMap<String, String>,
}

impl Matchers {
    /// True if no matcher field is set — i.e. this is a catch-all.
    pub fn is_empty(&self) -> bool {
        self.stream.is_none()
            && self.prompt_tokens_gte.is_none()
            && self.prompt_tokens_lte.is_none()
            && self.message_count_gte.is_none()
            && self.message_count_lte.is_none()
            && self.has_system_prompt.is_none()
            && self.content_regex.is_none()
            && self.header_equals.is_empty()
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetricsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_metrics_path")]
    pub path: String,
    /// When set, `/metrics` is served only from a dedicated `host:port`
    /// listener (typically loopback like `127.0.0.1:9091`) and the main
    /// listener no longer exposes it.
    #[serde(default)]
    pub bind: Option<String>,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_metrics_path(),
            bind: None,
        }
    }
}

fn default_metrics_path() -> String {
    "/metrics".to_string()
}

/// How to derive the VRAM budget in MB.
///
/// `Fixed(n)` uses `n` verbatim (0 = scheduling disabled, matching 2F behavior).
/// `Auto` queries NVML at startup and subtracts `vram_budget_headroom_mb`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VramBudgetSetting {
    Auto,
    Fixed(u64),
}

impl Default for VramBudgetSetting {
    fn default() -> Self {
        VramBudgetSetting::Fixed(0)
    }
}

impl<'de> serde::Deserialize<'de> for VramBudgetSetting {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(u64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Int(n) => Ok(VramBudgetSetting::Fixed(n)),
            Raw::Str(s) if s == "auto" => Ok(VramBudgetSetting::Auto),
            Raw::Str(s) => Err(D::Error::custom(format!(
                "vram_budget_mb: expected an integer or the string \"auto\", got {s:?}"
            ))),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Grace period after SIGTERM/Ctrl+C for in-flight work before workers are
    /// abandoned. Default 30s; range [0, 3600] (clamped with a warning).
    #[serde(default = "default_shutdown_grace_secs")]
    pub shutdown_grace_secs: u64,
    #[serde(default)]
    pub api_keys: Vec<String>,
    /// Allow binding publicly without `api_keys` (e.g. auth at a proxy).
    /// Default false: public binds without keys refuse to start.
    #[serde(default)]
    pub allow_unauthenticated: bool,
    /// CORS allow-list. Empty + loopback → permissive for local dev; empty +
    /// public bind → deny cross-origin; non-empty → used as-is.
    #[serde(default)]
    pub cors_origins: Vec<String>,
    /// Allow `http://`, loopback, and private-range cloud `base_url` values.
    /// Default false to block SSRF via mis-set env vars.
    #[serde(default)]
    pub allow_plaintext_upstream: bool,
    /// VRAM budget for local models. `"auto"` queries NVML and subtracts
    /// `vram_budget_headroom_mb`. An integer is used verbatim; `0` disables
    /// scheduling.
    #[serde(default)]
    pub vram_budget_mb: VramBudgetSetting,

    /// Only honored when `vram_budget_mb = "auto"`. Default 2048 MB.
    #[serde(default = "default_vram_headroom_mb")]
    pub vram_budget_headroom_mb: u64,

    /// Per-device VRAM budget override. Keys are gpu_ids, values are MB.
    /// An override wins over the default derived from `vram_budget_mb`
    /// (whether Auto or Fixed). Unlisted devices use the default.
    #[serde(default)]
    pub vram_budget_overrides: std::collections::HashMap<u32, u64>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            shutdown_grace_secs: default_shutdown_grace_secs(),
            api_keys: Vec::new(),
            allow_unauthenticated: false,
            cors_origins: Vec::new(),
            allow_plaintext_upstream: false,
            vram_budget_mb: VramBudgetSetting::Fixed(0),
            vram_budget_headroom_mb: default_vram_headroom_mb(),
            vram_budget_overrides: std::collections::HashMap::new(),
        }
    }
}

impl ServerConfig {
    /// True when `host` is a loopback address (drives auth/CORS defaults).
    ///
    /// We match the literal host string rather than DNS-resolving — an
    /// operator who sets a fancy hostname is expected to know whether it
    /// binds locally.
    pub fn binds_loopback(&self) -> bool {
        let h = self.host.trim();
        if h.eq_ignore_ascii_case("localhost") {
            return true;
        }
        if let Ok(ip) = h.parse::<std::net::IpAddr>() {
            return ip.is_loopback();
        }
        false
    }

    /// Resolve the configured `vram_budget_mb` to a concrete u64 for
    /// `ResidentSet::new`. `Fixed(n)` → n. `Auto` → NVML device 0 total MB
    /// minus `vram_budget_headroom_mb`.
    pub fn resolve_vram_budget_mb(&self) -> Result<u64, ConfigError> {
        match self.vram_budget_mb {
            VramBudgetSetting::Fixed(n) => Ok(n),
            VramBudgetSetting::Auto => {
                let info = crate::engine::vram_detect::detect_device_zero()
                    .map_err(|source| ConfigError::VramAutoDetectFailed { source })?;
                let resolved = Self::resolve_vram_budget_mb_from_info(
                    &info,
                    self.vram_budget_headroom_mb,
                )?;
                tracing::info!(
                    budget_mb = resolved,
                    source = "auto",
                    device = info.device_index,
                    total_mb = info.total_mb,
                    headroom_mb = self.vram_budget_headroom_mb,
                    "vram budget resolved"
                );
                Ok(resolved)
            }
        }
    }

    /// Pure-arithmetic helper split out for testing (no NVML call).
    pub fn resolve_vram_budget_mb_from_info(
        info: &crate::engine::vram_detect::VramInfo,
        headroom_mb: u64,
    ) -> Result<u64, ConfigError> {
        if info.total_mb <= headroom_mb {
            return Err(ConfigError::VramAutoDetectInsufficient {
                total_mb: info.total_mb,
                headroom_mb,
            });
        }
        Ok(info.total_mb - headroom_mb)
    }

    /// Resolve per-device VRAM budgets. Returns a `Vec<u64>` of length
    /// `device_count`, indexed by gpu_id.
    ///
    /// - `Fixed(n)`: returns `vec![n; declared_device_count]`.
    ///   Caller passes `declared_device_count` computed from
    ///   `max(1, 1 + max_gpu_id_referenced_in_models)`.
    /// - `Auto`: calls `detect_all_devices()`, ignores
    ///   `declared_device_count`, and returns one budget per detected
    ///   device (`total_mb - headroom`).
    ///   Applies `vram_budget_overrides` on top; an override for a gpu_id
    ///   beyond the device count → `VramOverrideUnknownGpu`.
    pub fn resolve_vram_budgets(
        &self,
        declared_device_count: u32,
    ) -> Result<Vec<u64>, ConfigError> {
        let mut budgets: Vec<u64> = match self.vram_budget_mb {
            VramBudgetSetting::Fixed(n) => {
                vec![n; declared_device_count as usize]
            }
            VramBudgetSetting::Auto => {
                let infos = crate::engine::vram_detect::detect_all_devices()
                    .map_err(|source| ConfigError::VramAutoDetectFailed { source })?;
                let mut out = Vec::with_capacity(infos.len());
                for info in &infos {
                    let b = Self::resolve_vram_budget_mb_from_info(
                        info,
                        self.vram_budget_headroom_mb,
                    )?;
                    tracing::info!(
                        budget_mb = b,
                        source = "auto",
                        device = info.device_index,
                        total_mb = info.total_mb,
                        headroom_mb = self.vram_budget_headroom_mb,
                        "vram budget resolved (per-device)"
                    );
                    out.push(b);
                }
                out
            }
        };

        let device_count = budgets.len() as u32;

        // Apply per-device overrides.
        for (&gpu_id, &mb) in &self.vram_budget_overrides {
            if gpu_id >= device_count {
                return Err(ConfigError::VramOverrideUnknownGpu {
                    gpu_id,
                    device_count,
                });
            }
            budgets[gpu_id as usize] = mb;
        }

        Ok(budgets)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub backend: BackendType,

    // Local-only
    pub path: Option<PathBuf>,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default = "default_gpu_layers")]
    pub gpu_layers: u32,
    pub threads: Option<u32>,
    pub batch_size: Option<u32>,
    pub seed: Option<u32>,

    // Cloud-only
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub upstream_model: Option<String>,
    pub timeout_secs: Option<u64>,

    /// Upper bound on `max_tokens` for this model (silent clamp). `None` →
    /// global default (8192).
    #[serde(default)]
    pub max_tokens_cap: Option<u32>,

    /// Defer load until first request (cold-start). Local backends only.
    #[serde(default)]
    pub lazy: bool,

    /// Override estimated VRAM in MB (default: file size × 1.2). Only when
    /// `vram_budget_mb` is set; local backends only.
    #[serde(default)]
    pub vram_mb: Option<u64>,

    /// Never evict this model from VRAM under budget pressure. Local backends
    /// only. Pinned models count against `vram_budget_mb` at startup.
    #[serde(default)]
    pub pin: bool,

    /// GPU placement for local backends.
    /// - `[]` (default) = auto-placement (scheduler best-fits at first load).
    /// - `[N]` = pin to device N.
    /// - `[N, M, ...]` (len ≥ 2) = tensor-parallel split across those devices.
    #[serde(default)]
    pub gpus: Vec<u32>,

    // HF-only
    /// HF Hub repo id (e.g. `"Qwen/Qwen2.5-32B-Instruct"`). Mutually exclusive
    /// with `path`. HF backend only.
    pub repo: Option<String>,
    /// Git revision / tag for the HF Hub repo. Defaults to `"main"` when unset.
    pub revision: Option<String>,
    /// Precision / quantization selection for the HF backend. Unset → default
    /// picked by the backend at load time (bf16 if device supports it, else fp16).
    pub dtype: Option<Dtype>,
    /// Name of the environment variable holding the HF Hub token. Used for gated
    /// repos. HF backend only.
    pub hf_token_env: Option<String>,
    /// LoRA adapters to merge at load time. HF backend only.
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    Local,
    Openai,
    Groq,
    Anthropic,
    Hf,
}

/// Precision / quantization knob for the HF backend.
///
/// `bf16` / `fp16` load safetensors as-is. Q4/Q8 variants use Candle's
/// `quantized` module (GGUF-flavor quantization).
#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dtype {
    Bf16,
    Fp16,
    #[serde(rename = "q4_0")]
    Q4_0,
    #[serde(rename = "q4_k_m")]
    Q4KM,
    #[serde(rename = "q8_0")]
    Q8_0,
}

/// LoRA adapter loaded at model load time (merge-only in Wave 7).
///
/// Exactly one of `path` or `repo` must be set. Scale defaults to `1.0`.
#[derive(Debug, Deserialize, Clone)]
pub struct AdapterConfig {
    pub path: Option<PathBuf>,
    pub repo: Option<String>,
    pub revision: Option<String>,
    #[serde(default = "default_adapter_scale")]
    pub scale: f32,
}

fn default_adapter_scale() -> f32 {
    1.0
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

fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    8080
}
fn default_context_size() -> u32 {
    4096
}
fn default_gpu_layers() -> u32 {
    99
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_shutdown_grace_secs() -> u64 {
    30
}
fn default_vram_headroom_mb() -> u64 {
    2048
}

/// Known provider-default base URLs. Used by SSRF validation when a cloud
/// model has `base_url` unset — we still check the resolved URL the backend
/// will actually hit.
fn provider_default_base_url(backend: &BackendType) -> Option<&'static str> {
    match backend {
        BackendType::Openai => Some("https://api.openai.com/v1"),
        BackendType::Groq => Some("https://api.groq.com/openai/v1"),
        BackendType::Anthropic => Some("https://api.anthropic.com/v1"),
        BackendType::Local => None,
        BackendType::Hf => None,
    }
}

/// True when an IPv6 address is in an RFC-4193 unique-local (fc00::/7) or
/// RFC-4291 link-local (fe80::/10) range.
fn is_ipv6_private(ip: std::net::Ipv6Addr) -> bool {
    let seg = ip.segments()[0];
    let is_ula = (seg & 0xfe00) == 0xfc00;
    let is_link_local = (seg & 0xffc0) == 0xfe80;
    is_ula || is_link_local
}

/// Reject plaintext schemes, loopback, link-local, and RFC-1918 private-range
/// hosts unless the operator opts in via `[server].allow_plaintext_upstream`.
fn validate_upstream_url(
    id: &str,
    url_str: &str,
    allow_plaintext: bool,
) -> Result<(), ConfigError> {
    let parsed = url::Url::parse(url_str).map_err(|e| ConfigError::InvalidBaseUrl {
        id: id.into(),
        reason: e.to_string(),
    })?;

    let scheme = parsed.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(ConfigError::InvalidBaseUrl {
            id: id.into(),
            reason: format!("unsupported scheme '{scheme}' (only http and https are allowed)"),
        });
    }

    if scheme == "http" && !allow_plaintext {
        return Err(ConfigError::PlaintextUpstreamForbidden {
            id: id.into(),
            url: url_str.into(),
        });
    }

    if let Some(host) = parsed.host() {
        let dangerous = match host {
            url::Host::Ipv4(ip) => {
                ip.is_loopback() || ip.is_link_local() || ip.is_private() || ip.is_unspecified()
            }
            url::Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified() || is_ipv6_private(ip),
            url::Host::Domain(d) => d.eq_ignore_ascii_case("localhost") || d.is_empty(),
        };
        if dangerous && !allow_plaintext {
            return Err(ConfigError::PlaintextUpstreamForbidden {
                id: id.into(),
                url: url_str.into(),
            });
        }
    } else {
        return Err(ConfigError::InvalidBaseUrl {
            id: id.into(),
            reason: "URL has no host component".into(),
        });
    }

    Ok(())
}

fn reject_hf_fields_on_non_hf(m: &ModelConfig) -> Result<(), ConfigError> {
    let bad_field = if m.repo.is_some() { Some("repo") }
        else if m.revision.is_some() { Some("revision") }
        else if m.dtype.is_some() { Some("dtype") }
        else if m.hf_token_env.is_some() { Some("hf_token_env") }
        else if !m.adapters.is_empty() { Some("adapters") }
        else { None };
    if let Some(field) = bad_field {
        return Err(ConfigError::HfFieldOnNonHfBackend {
            id: m.id.clone(),
            field: field.into(),
        });
    }
    Ok(())
}

impl FlarionConfig {
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::ReadFailed {
            path: path.to_path_buf(),
            source: e,
        })?;
        let mut config: FlarionConfig =
            toml::from_str(&content).map_err(|e| ConfigError::ParseFailed {
                path: path.to_path_buf(),
                source: e,
            })?;
        interpolate_env(&mut config)?;
        Ok(config)
    }

    /// Validate the config according to sub-phase 2b rules.
    /// Errors in this order:
    ///   1. At least one model declared
    ///   2. All ids are non-empty
    ///   3. All ids are unique
    ///   4. For backend = "local": path is set and exists
    ///   5. For non-local backends: api_key is set and non-empty, path is NOT set
    pub fn validate(&mut self) -> Result<(), ConfigError> {
        // Clamp shutdown_grace_secs with a warning if out of range.
        if self.server.shutdown_grace_secs > 3600 {
            tracing::warn!(
                requested = self.server.shutdown_grace_secs,
                "shutdown_grace_secs exceeds 3600; clamping to 3600"
            );
            self.server.shutdown_grace_secs = 3600;
        }

        if self.models.is_empty() {
            return Err(ConfigError::NoModelsConfigured);
        }

        for m in &self.models {
            if m.id.trim().is_empty() {
                return Err(ConfigError::EmptyModelId);
            }
        }

        let mut seen: HashSet<&str> = HashSet::new();
        for m in &self.models {
            if !seen.insert(m.id.as_str()) {
                return Err(ConfigError::DuplicateModelId { id: m.id.clone() });
            }
        }

        for m in &self.models {
            match m.backend {
                BackendType::Local => {
                    reject_hf_fields_on_non_hf(m)?;
                    let path = m
                        .path
                        .as_ref()
                        .ok_or_else(|| ConfigError::LocalBackendNeedsPath { id: m.id.clone() })?;
                    if !path.is_file() {
                        return Err(ConfigError::ModelPathMissing {
                            id: m.id.clone(),
                            path: path.clone(),
                        });
                    }
                }
                BackendType::Openai | BackendType::Groq | BackendType::Anthropic => {
                    reject_hf_fields_on_non_hf(m)?;
                    if m.path.is_some() {
                        return Err(ConfigError::PathOnCloudBackend { id: m.id.clone() });
                    }
                    let key = m
                        .api_key
                        .as_ref()
                        .ok_or_else(|| ConfigError::CloudBackendNeedsApiKey { id: m.id.clone() })?;
                    if key.trim().is_empty() {
                        return Err(ConfigError::CloudBackendNeedsApiKey { id: m.id.clone() });
                    }

                    // SSRF: validate the effective upstream URL (configured
                    // base_url or provider default).
                    let effective_url = m
                        .base_url
                        .as_deref()
                        .or_else(|| provider_default_base_url(&m.backend))
                        .ok_or_else(|| ConfigError::InvalidBaseUrl {
                            id: m.id.clone(),
                            reason: "no base_url and no provider default".into(),
                        })?;
                    validate_upstream_url(
                        &m.id,
                        effective_url,
                        self.server.allow_plaintext_upstream,
                    )?;
                }
                BackendType::Hf => {
                    // Exactly one of path / repo.
                    match (&m.path, &m.repo) {
                        (Some(_), Some(_)) => {
                            return Err(ConfigError::HfBackendPathAndRepoExclusive {
                                id: m.id.clone(),
                            });
                        }
                        (None, None) => {
                            return Err(ConfigError::HfBackendNeedsPathOrRepo {
                                id: m.id.clone(),
                            });
                        }
                        _ => {}
                    }

                    // Each adapter: exactly one of path / repo.
                    for (index, a) in m.adapters.iter().enumerate() {
                        match (&a.path, &a.repo) {
                            (Some(_), Some(_)) => {
                                return Err(ConfigError::HfAdapterPathAndRepoExclusive {
                                    id: m.id.clone(),
                                    index,
                                });
                            }
                            (None, None) => {
                                return Err(ConfigError::HfAdapterNeedsPathOrRepo {
                                    id: m.id.clone(),
                                    index,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let model_ids: HashSet<&str> = self.models.iter().map(|m| m.id.as_str()).collect();
        let mut seen_routes: HashSet<&str> = HashSet::new();

        for route in &self.routes {
            if route.id.trim().is_empty() {
                return Err(ConfigError::RouteEmptyId);
            }
            if !seen_routes.insert(route.id.as_str()) {
                return Err(ConfigError::DuplicateRouteId {
                    id: route.id.clone(),
                });
            }
            if model_ids.contains(route.id.as_str()) {
                return Err(ConfigError::RouteIdCollision {
                    id: route.id.clone(),
                });
            }
            if route.rules.is_empty() {
                return Err(ConfigError::RouteNoRules {
                    id: route.id.clone(),
                });
            }

            let route_ids: HashSet<&str> = self.routes.iter().map(|r| r.id.as_str()).collect();
            for rule in &route.rules {
                if rule.targets.is_empty() {
                    return Err(ConfigError::RuleNoTargets {
                        route_id: route.id.clone(),
                        rule: rule.name.clone(),
                    });
                }
                for target in &rule.targets {
                    if route_ids.contains(target.as_str()) {
                        return Err(ConfigError::RouteTargetsRoute {
                            route_id: route.id.clone(),
                            target: target.clone(),
                        });
                    }
                    if !model_ids.contains(target.as_str()) {
                        return Err(ConfigError::RouteTargetUnknown {
                            route_id: route.id.clone(),
                            target: target.clone(),
                        });
                    }
                }
                if let Some(ref pattern) = rule.matchers.content_regex
                    && let Err(e) = regex::Regex::new(pattern)
                {
                    return Err(ConfigError::InvalidRegex {
                        route_id: route.id.clone(),
                        rule: rule.name.clone(),
                        error: e.to_string(),
                    });
                }
            }

            let has_catch_all = route.rules.iter().any(|r| r.matchers.is_empty());
            if !has_catch_all {
                return Err(ConfigError::RouteNoCatchAll {
                    id: route.id.clone(),
                });
            }
        }

        for origin in &self.server.cors_origins {
            if let Err(e) = url::Url::parse(origin) {
                tracing::debug!(error = %e, origin = %origin, "cors origin parse error");
                return Err(ConfigError::InvalidCorsOrigin {
                    origin: origin.clone(),
                });
            }
        }

        if let Some(ref addr) = self.metrics.bind
            && addr.parse::<std::net::SocketAddr>().is_err()
        {
            return Err(ConfigError::InvalidMetricsBind { addr: addr.clone() });
        }

        for m in &self.models {
            if m.lazy && m.backend != BackendType::Local {
                return Err(ConfigError::LazyOnlyForLocal {
                    id: m.id.clone(),
                    backend: m.backend.clone(),
                });
            }
            if m.vram_mb.is_some() && m.backend != BackendType::Local {
                return Err(ConfigError::VramMbOnlyForLocal {
                    id: m.id.clone(),
                    backend: m.backend.clone(),
                });
            }
            if m.pin && m.backend != BackendType::Local {
                return Err(ConfigError::PinOnlyForLocal {
                    id: m.id.clone(),
                    backend: m.backend.clone(),
                });
            }
            if !m.gpus.is_empty() && m.backend != BackendType::Local {
                return Err(ConfigError::GpuIdOnCloudBackend {
                    id: m.id.clone(),
                    gpus: m.gpus.clone(),
                    backend: m.backend.clone(),
                });
            }
            // Duplicate-gpu check.
            let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
            for &gpu_id in &m.gpus {
                if !seen.insert(gpu_id) {
                    return Err(ConfigError::GpuIdDuplicated {
                        model_id: m.id.clone(),
                        gpu_id,
                    });
                }
            }
        }

        // Phase 2H: compute declared device count and per-device budgets.
        let declared_device_count = self
            .models
            .iter()
            .flat_map(|m| m.gpus.iter().copied())
            .max()
            .map(|m| m + 1)
            .unwrap_or(1);

        let budgets = self.server.resolve_vram_budgets(declared_device_count)?;
        let device_count = budgets.len() as u32;

        // Only validate gpu_ids for local backends (already filtered on
        // backend type by GpuIdOnCloudBackend earlier).
        for m in &self.models {
            if m.backend != BackendType::Local {
                continue;
            }
            for &gpu_id in &m.gpus {
                if gpu_id >= device_count {
                    return Err(ConfigError::GpuIdExceedsDetected {
                        model_id: m.id.clone(),
                        gpu_id,
                        detected_count: device_count,
                    });
                }
            }
        }

        // Per-device eager overflow check.
        let mut eager_totals: std::collections::HashMap<u32, (u64, Vec<(String, u64)>)> =
            std::collections::HashMap::new();
        for m in &self.models {
            if m.backend != BackendType::Local || m.lazy {
                continue;
            }
            let path = m
                .path
                .as_ref()
                .expect("local backend path must be set — earlier validation ensures this");
            let est = crate::engine::scheduling::estimate_vram_mb(path, m.vram_mb)
                .map_err(|e| match e {
                    crate::engine::scheduling::EstimateError::StatFailed { path, source } => {
                        ConfigError::VramEstimateFailed {
                            id: m.id.clone(),
                            path,
                            source,
                        }
                    }
                })?;
            // Resolve placement. Models with gpus=[] are auto-placed — they
            // don't yet know their device at validation time, so we skip
            // them in per-device eager checks (runtime best-fit handles it).
            let placement = crate::engine::scheduling::ResolvedPlacement::from_gpus(&m.gpus);
            if let crate::engine::scheduling::ResolvedPlacement::Resolved(p) = placement {
                for (gpu_id, cost) in p.per_device_cost(est) {
                    let entry = eager_totals
                        .entry(gpu_id)
                        .or_insert((0, Vec::new()));
                    entry.0 = entry.0.saturating_add(cost);
                    entry.1.push((m.id.clone(), cost));
                }
            }
        }
        // Find any gpu where eager total > budget. Deterministic order by gpu_id.
        let mut eager_gpus: Vec<u32> = eager_totals.keys().copied().collect();
        eager_gpus.sort();
        for gpu_id in eager_gpus {
            let (total_mb, offenders) = &eager_totals[&gpu_id];
            let budget_mb = budgets[gpu_id as usize];
            if budget_mb > 0 && *total_mb > budget_mb {
                return Err(ConfigError::EagerLoadsExceedBudget {
                    gpu_id,
                    total_mb: *total_mb,
                    budget_mb,
                    offenders: offenders.clone(),
                });
            }
        }

        // Per-device pinned overflow check.
        let mut pinned_totals: std::collections::HashMap<u32, (u64, Vec<(String, u64)>)> =
            std::collections::HashMap::new();
        for m in &self.models {
            if m.backend != BackendType::Local || !m.pin {
                continue;
            }
            let path = m
                .path
                .as_ref()
                .expect("local backend path must be set — earlier validation ensures this");
            let est = crate::engine::scheduling::estimate_vram_mb(path, m.vram_mb)
                .map_err(|e| match e {
                    crate::engine::scheduling::EstimateError::StatFailed { path, source } => {
                        ConfigError::VramEstimateFailed {
                            id: m.id.clone(),
                            path,
                            source,
                        }
                    }
                })?;
            let placement = crate::engine::scheduling::ResolvedPlacement::from_gpus(&m.gpus);
            if let crate::engine::scheduling::ResolvedPlacement::Resolved(p) = placement {
                for (gpu_id, cost) in p.per_device_cost(est) {
                    let entry = pinned_totals
                        .entry(gpu_id)
                        .or_insert((0, Vec::new()));
                    entry.0 = entry.0.saturating_add(cost);
                    entry.1.push((m.id.clone(), cost));
                }
            }
        }
        let mut pinned_gpus: Vec<u32> = pinned_totals.keys().copied().collect();
        pinned_gpus.sort();
        for gpu_id in pinned_gpus {
            let (total_mb, offenders) = &pinned_totals[&gpu_id];
            let budget_mb = budgets[gpu_id as usize];
            if budget_mb > 0 && *total_mb > budget_mb {
                return Err(ConfigError::PinnedExceedsBudget {
                    gpu_id,
                    total_mb: *total_mb,
                    budget_mb,
                    offenders: offenders.clone(),
                });
            }
        }

        Ok(())
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

/// Replace `${VAR_NAME}` substrings with environment variable values.
/// Only operates on string fields known to allow secrets / endpoints:
///   - server.api_keys[*]
///   - models[*].api_key
///   - models[*].base_url
///
/// Missing env var → ConfigError::MissingEnvVar with the variable name and field path.
pub fn interpolate_env(config: &mut FlarionConfig) -> Result<(), ConfigError> {
    for (i, key) in config.server.api_keys.iter_mut().enumerate() {
        let field = format!("server.api_keys[{i}]");
        *key = expand_string(key, &field)?;
    }

    for (i, model) in config.models.iter_mut().enumerate() {
        if let Some(ref mut api_key) = model.api_key {
            let field = format!("models[{i}].api_key");
            *api_key = expand_string(api_key, &field)?;
        }
        if let Some(ref mut base_url) = model.base_url {
            let field = format!("models[{i}].base_url");
            *base_url = expand_string(base_url, &field)?;
        }
    }

    Ok(())
}

/// Replace every `${VAR}` occurrence in `input` with the value of `$VAR`.
/// Returns ConfigError::MissingEnvVar on the first missing variable.
fn expand_string(input: &str, field: &str) -> Result<String, ConfigError> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after_dollar = &rest[start + 2..];
        let end = after_dollar
            .find('}')
            .ok_or_else(|| ConfigError::MalformedEnvRef {
                field: field.to_string(),
                text: input.to_string(),
            })?;
        let var_name = &after_dollar[..end];
        if !is_valid_env_name(var_name) {
            return Err(ConfigError::MalformedEnvRef {
                field: field.to_string(),
                text: input.to_string(),
            });
        }
        let value = std::env::var(var_name).map_err(|_| ConfigError::MissingEnvVar {
            var: var_name.to_string(),
            field: field.to_string(),
        })?;
        out.push_str(&value);
        rest = &after_dollar[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase() || c == '_')
            .unwrap_or(false)
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
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
    #[error("no models configured — declare at least one [[models]] entry")]
    NoModelsConfigured,
    #[error("model id must be non-empty")]
    EmptyModelId,
    #[error("duplicate model id '{id}'")]
    DuplicateModelId { id: String },
    #[error("model '{id}': path '{}' does not exist", path.display())]
    ModelPathMissing { id: String, path: PathBuf },
    #[error("model '{id}': local backend requires a `path` field")]
    LocalBackendNeedsPath { id: String },
    #[error("model '{id}': cloud backend requires an `api_key` field")]
    CloudBackendNeedsApiKey { id: String },
    #[error("model '{id}': `path` field is only valid for local backend")]
    PathOnCloudBackend { id: String },
    #[error("missing env var '{var}' referenced by {field}")]
    MissingEnvVar { var: String, field: String },
    #[error("malformed env reference in {field}: '{text}'")]
    MalformedEnvRef { field: String, text: String },
    #[error("route id must be non-empty")]
    RouteEmptyId,
    #[error("duplicate route id '{id}'")]
    DuplicateRouteId { id: String },
    #[error("route id '{id}' collides with a model id")]
    RouteIdCollision { id: String },
    #[error("route '{id}' has no rules — declare at least one [[routes.rules]] entry")]
    RouteNoRules { id: String },
    #[error("route '{route_id}' rule '{rule}' has no targets")]
    RuleNoTargets { route_id: String, rule: String },
    #[error("route '{route_id}' target '{target}' is not a known model id")]
    RouteTargetUnknown { route_id: String, target: String },
    #[error(
        "route '{route_id}' target '{target}' points at another route — routes cannot target routes"
    )]
    RouteTargetsRoute { route_id: String, target: String },
    #[error(
        "route '{id}' has no catch-all rule — add a rule with empty `matchers = {{}}` at the end"
    )]
    RouteNoCatchAll { id: String },
    #[error("route '{route_id}' rule '{rule}' has invalid regex: {error}")]
    InvalidRegex {
        route_id: String,
        rule: String,
        error: String,
    },
    #[error(
        "model '{id}' base_url '{url}' uses plaintext or private-range host; set [server].allow_plaintext_upstream = true to permit"
    )]
    PlaintextUpstreamForbidden { id: String, url: String },

    #[error("model '{id}' has invalid base_url: {reason}")]
    InvalidBaseUrl { id: String, reason: String },

    #[error("[metrics].bind '{addr}' is not a valid socket address")]
    InvalidMetricsBind { addr: String },

    #[error("[server].cors_origins entry '{origin}' is not a valid URL")]
    InvalidCorsOrigin { origin: String },

    #[error(
        "server is binding to a public address ({host}) without authentication; set [server].api_keys = [...] or [server].allow_unauthenticated = true to start anyway"
    )]
    PublicBindRequiresAuth { host: String },

    #[error(
        "model '{id}' has lazy=true but backend={backend:?}; lazy is only supported for local backends"
    )]
    LazyOnlyForLocal { id: String, backend: BackendType },

    #[error(
        "model '{id}' has vram_mb set but backend={backend:?}; vram_mb is only supported for local backends"
    )]
    VramMbOnlyForLocal { id: String, backend: BackendType },

    #[error(
        "eager local models on gpu {gpu_id} total {total_mb}MB, exceeds budget={budget_mb}; offenders: {offenders:?}"
    )]
    EagerLoadsExceedBudget {
        gpu_id: u32,
        total_mb: u64,
        budget_mb: u64,
        offenders: Vec<(String, u64)>,
    },

    #[error("failed to estimate VRAM for model '{id}' at {path}: {source}")]
    VramEstimateFailed {
        id: String,
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "model '{id}' has pin=true but backend={backend:?}; pin is only supported for local backends"
    )]
    PinOnlyForLocal { id: String, backend: BackendType },

    #[error(
        "pinned local models on gpu {gpu_id} total {total_mb}MB, exceeds budget={budget_mb}; offenders: {offenders:?}"
    )]
    PinnedExceedsBudget {
        gpu_id: u32,
        total_mb: u64,
        budget_mb: u64,
        offenders: Vec<(String, u64)>,
    },

    #[error(
        "model '{model_id}' has duplicate gpu {gpu_id} in placement"
    )]
    GpuIdDuplicated { model_id: String, gpu_id: u32 },

    #[error(
        "model '{id}' has gpus={gpus:?} but backend={backend:?}; gpus is only supported for local backends"
    )]
    GpuIdOnCloudBackend {
        id: String,
        gpus: Vec<u32>,
        backend: BackendType,
    },

    #[error("auto VRAM detection failed: {source}; set vram_budget_mb to an explicit integer MB or 0 to disable")]
    VramAutoDetectFailed {
        #[source]
        source: crate::engine::vram_detect::VramDetectError,
    },

    #[error("detected VRAM ({total_mb}MB) is <= headroom ({headroom_mb}MB); reduce vram_budget_headroom_mb")]
    VramAutoDetectInsufficient { total_mb: u64, headroom_mb: u64 },

    #[error(
        "vram_budget_overrides references gpu {gpu_id} but only {device_count} device(s) detected/declared"
    )]
    VramOverrideUnknownGpu { gpu_id: u32, device_count: u32 },

    #[error(
        "model '{model_id}' references gpu {gpu_id} but only {detected_count} device(s) detected"
    )]
    GpuIdExceedsDetected {
        model_id: String,
        gpu_id: u32,
        detected_count: u32,
    },

    #[error("model '{id}': HF backend requires exactly one of `path` or `repo`")]
    HfBackendNeedsPathOrRepo { id: String },
    #[error("model '{id}': HF backend cannot set both `path` and `repo`")]
    HfBackendPathAndRepoExclusive { id: String },
    #[error("model '{id}': field `{field}` is only valid on HF backend")]
    HfFieldOnNonHfBackend { id: String, field: String },
    #[error("model '{id}': adapter #{index} requires exactly one of `path` or `repo`")]
    HfAdapterNeedsPathOrRepo { id: String, index: usize },
    #[error("model '{id}': adapter #{index} cannot set both `path` and `repo`")]
    HfAdapterPathAndRepoExclusive { id: String, index: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_cfg(id: &str, path: PathBuf) -> ModelConfig {
        ModelConfig {
            id: id.into(),
            backend: BackendType::Local,
            path: Some(path),
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: None,
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
            gpus: vec![],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }
    }

    fn cloud_cfg(id: &str, backend: BackendType, api_key: Option<&str>) -> ModelConfig {
        ModelConfig {
            id: id.into(),
            backend,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: api_key.map(String::from),
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
            gpus: vec![],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }
    }

    #[test]
    fn test_parse_minimal_multi_model_config() {
        let toml_str = r#"
[server]

[[models]]
id = "test-model"
backend = "local"
path = "/tmp/model.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].id, "test-model");
        assert_eq!(config.models[0].backend, BackendType::Local);
        assert_eq!(config.models[0].context_size, 4096);
        assert_eq!(config.models[0].gpu_layers, 99);
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_parse_two_model_config() {
        let toml_str = r#"
[server]
host = "127.0.0.1"
port = 3000

[[models]]
id = "qwen3-8b"
backend = "local"
path = "/models/qwen3-8b.gguf"
context_size = 8192
gpu_layers = 40
threads = 8
batch_size = 512
seed = 42

[[models]]
id = "codellama-13b"
backend = "local"
path = "/models/codellama-13b.gguf"

[logging]
level = "debug"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.models.len(), 2);

        let qwen = &config.models[0];
        assert_eq!(qwen.id, "qwen3-8b");
        assert_eq!(qwen.context_size, 8192);
        assert_eq!(qwen.gpu_layers, 40);
        assert_eq!(qwen.threads, Some(8));
        assert_eq!(qwen.batch_size, Some(512));
        assert_eq!(qwen.seed, Some(42));

        let codellama = &config.models[1];
        assert_eq!(codellama.id, "codellama-13b");
        assert_eq!(codellama.context_size, 4096);
        assert_eq!(codellama.gpu_layers, 99);

        assert_eq!(config.logging.level, "debug");
    }

    #[test]
    fn test_missing_backend_field_errors() {
        let toml_str = r#"
[server]

[[models]]
id = "test"
path = "/tmp/model.gguf"
"#;
        let result: Result<FlarionConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "expected error for missing backend field");
    }

    #[test]
    fn test_validate_accepts_good_config() {
        let tmp = std::env::temp_dir().join("flarion-test-model.gguf");
        std::fs::write(&tmp, b"").unwrap();

        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![local_cfg("test", tmp.clone())],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        assert!(config.validate().is_ok());

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_empty_models() {
        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: Vec::new(),
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::NoModelsConfigured),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_validate_rejects_duplicate_ids() {
        let tmp = std::env::temp_dir().join("flarion-test-dup.gguf");
        std::fs::write(&tmp, b"").unwrap();

        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![local_cfg("dup", tmp.clone()), local_cfg("dup", tmp.clone())],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::DuplicateModelId { .. }),
            "got: {err:?}"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_empty_id() {
        let tmp = std::env::temp_dir().join("flarion-test-empty.gguf");
        std::fs::write(&tmp, b"").unwrap();

        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![local_cfg("", tmp.clone())],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::EmptyModelId), "got: {err:?}");

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_missing_local_path() {
        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![local_cfg(
                "test",
                PathBuf::from("/definitely/does/not/exist.gguf"),
            )],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::ModelPathMissing { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_cli_overrides() {
        let toml_str = r#"
[server]
host = "0.0.0.0"
port = 8080

[[models]]
id = "test"
backend = "local"
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

    #[test]
    fn test_parse_openai_backend() {
        let toml_str = r#"
[server]

[[models]]
id = "gpt-4o"
backend = "openai"
api_key = "sk-test"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.models[0].backend, BackendType::Openai);
        assert_eq!(config.models[0].api_key.as_deref(), Some("sk-test"));
        assert!(config.models[0].path.is_none());
    }

    #[test]
    fn test_parse_anthropic_backend_with_options() {
        let toml_str = r#"
[server]

[[models]]
id = "claude-sonnet"
backend = "anthropic"
api_key = "sk-ant-test"
upstream_model = "claude-sonnet-4-5-20250929"
base_url = "https://example.com/v1"
timeout_secs = 600
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        let m = &config.models[0];
        assert_eq!(m.backend, BackendType::Anthropic);
        assert_eq!(
            m.upstream_model.as_deref(),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(m.base_url.as_deref(), Some("https://example.com/v1"));
        assert_eq!(m.timeout_secs, Some(600));
    }

    #[test]
    fn test_parse_server_api_keys() {
        let toml_str = r#"
[server]
api_keys = ["key1", "key2"]

[[models]]
id = "x"
backend = "local"
path = "/tmp/x.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(
            config.server.api_keys,
            vec!["key1".to_string(), "key2".to_string()]
        );
    }

    #[test]
    fn test_server_api_keys_default_empty() {
        let toml_str = r#"
[server]

[[models]]
id = "x"
backend = "local"
path = "/tmp/x.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert!(config.server.api_keys.is_empty());
    }

    #[test]
    fn test_validate_local_without_path_errors() {
        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![ModelConfig {
                id: "test".into(),
                backend: BackendType::Local,
                path: None,
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: false,
                vram_mb: None,
                pin: false,
                gpus: vec![],
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
            }],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::LocalBackendNeedsPath { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_validate_cloud_without_api_key_errors() {
        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![cloud_cfg("gpt-4o", BackendType::Openai, None)],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::CloudBackendNeedsApiKey { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_validate_cloud_with_path_errors() {
        let tmp = std::env::temp_dir().join("flarion-cloud-path.gguf");
        std::fs::write(&tmp, b"").unwrap();

        let mut model = cloud_cfg("gpt-4o", BackendType::Openai, Some("sk-test"));
        model.path = Some(tmp.clone());

        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![model],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::PathOnCloudBackend { .. }),
            "got: {err:?}"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_interpolate_env_replaces_var() {
        unsafe { std::env::set_var("FLARION_TEST_API_KEY", "actual-secret-value") };

        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![{
                let mut c = cloud_cfg("x", BackendType::Openai, Some("${FLARION_TEST_API_KEY}"));
                c.api_key = Some("${FLARION_TEST_API_KEY}".into());
                c
            }],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };

        interpolate_env(&mut config).unwrap();
        assert_eq!(
            config.models[0].api_key.as_deref(),
            Some("actual-secret-value")
        );

        unsafe { std::env::remove_var("FLARION_TEST_API_KEY") };
    }

    #[test]
    fn test_interpolate_env_missing_var_errors() {
        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![cloud_cfg(
                "x",
                BackendType::Openai,
                Some("${FLARION_DEFINITELY_UNSET_VAR_XYZ}"),
            )],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };

        let err = interpolate_env(&mut config).unwrap_err();
        match err {
            ConfigError::MissingEnvVar { ref var, ref field } => {
                assert_eq!(var, "FLARION_DEFINITELY_UNSET_VAR_XYZ");
                assert_eq!(field, "models[0].api_key");
            }
            other => panic!("expected MissingEnvVar, got: {other:?}"),
        }
    }

    #[test]
    fn test_interpolate_env_partial_string() {
        unsafe { std::env::set_var("FLARION_TEST_HOST", "https://api.example.com") };

        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![{
                let mut c = cloud_cfg("x", BackendType::Openai, Some("sk-literal"));
                c.base_url = Some("${FLARION_TEST_HOST}/v1".into());
                c
            }],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };

        interpolate_env(&mut config).unwrap();
        assert_eq!(
            config.models[0].base_url.as_deref(),
            Some("https://api.example.com/v1")
        );
        assert_eq!(config.models[0].api_key.as_deref(), Some("sk-literal"));

        unsafe { std::env::remove_var("FLARION_TEST_HOST") };
    }

    #[test]
    fn test_interpolate_env_no_placeholder_unchanged() {
        let mut config = FlarionConfig {
            server: ServerConfig::default(),
            models: vec![cloud_cfg(
                "x",
                BackendType::Openai,
                Some("sk-literal-value"),
            )],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };

        interpolate_env(&mut config).unwrap();
        assert_eq!(
            config.models[0].api_key.as_deref(),
            Some("sk-literal-value")
        );
    }

    #[test]
    fn test_interpolate_env_in_server_api_keys() {
        unsafe { std::env::set_var("FLARION_TEST_KEY1", "team-key-abc") };

        let mut config = FlarionConfig {
            server: ServerConfig {
                host: "0.0.0.0".into(),
                port: 8080,
                api_keys: vec!["${FLARION_TEST_KEY1}".into(), "literal-key".into()],
                ..ServerConfig::default()
            },
            models: vec![local_cfg(
                "x",
                std::env::temp_dir().join("flarion-fake.gguf"),
            )],
            logging: LoggingConfig::default(),
            ..FlarionConfig::default()
        };

        interpolate_env(&mut config).unwrap();
        assert_eq!(config.server.api_keys[0], "team-key-abc");
        assert_eq!(config.server.api_keys[1], "literal-key");

        unsafe { std::env::remove_var("FLARION_TEST_KEY1") };
    }

    #[test]
    fn test_parse_minimal_route_config() {
        let toml_str = r#"
[server]

[[models]]
id = "local"
backend = "local"
path = "/tmp/local.gguf"

[[routes]]
id = "chat"

  [[routes.rules]]
  name = "default"
  matchers = {}
  targets = ["local"]
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].id, "chat");
        assert_eq!(config.routes[0].rules.len(), 1);
        assert_eq!(config.routes[0].rules[0].targets, vec!["local".to_string()]);
        assert!(config.routes[0].rules[0].matchers.is_empty());
    }

    #[test]
    fn test_parse_full_route_with_matchers() {
        let toml_str = r#"
[server]

[[models]]
id = "local"
backend = "local"
path = "/tmp/local.gguf"

[[routes]]
id = "chat"
first_token_timeout_ms = 5000

  [[routes.rules]]
  name = "long-prompt"
  matchers = { prompt_tokens_gte = 4000, has_system_prompt = true }
  targets = ["local"]
  first_token_timeout_ms = 3000

  [[routes.rules]]
  name = "fallback"
  matchers = {}
  targets = ["local"]
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        let rule = &config.routes[0].rules[0];
        assert_eq!(rule.matchers.prompt_tokens_gte, Some(4000));
        assert_eq!(rule.matchers.has_system_prompt, Some(true));
        assert_eq!(rule.first_token_timeout_ms, Some(3000));
        assert_eq!(config.routes[0].first_token_timeout_ms, Some(5000));
    }

    #[test]
    fn test_parse_metrics_config_defaults() {
        let toml_str = r#"
[server]

[[models]]
id = "x"
backend = "local"
path = "/tmp/x.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.metrics.enabled);
        assert_eq!(config.metrics.path, "/metrics");
    }

    #[test]
    fn test_parse_metrics_enabled() {
        let toml_str = r#"
[server]

[[models]]
id = "x"
backend = "local"
path = "/tmp/x.gguf"

[metrics]
enabled = true
path = "/m"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert!(config.metrics.enabled);
        assert_eq!(config.metrics.path, "/m");
    }

    #[test]
    fn test_parse_header_equals_matcher() {
        let toml_str = r#"
[server]

[[models]]
id = "local"
backend = "local"
path = "/tmp/local.gguf"

[[routes]]
id = "chat"

  [[routes.rules]]
  name = "fast"
  matchers = { header_equals = { "X-Flarion-Route" = "fast" } }
  targets = ["local"]

  [[routes.rules]]
  name = "default"
  matchers = {}
  targets = ["local"]
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        let headers = &config.routes[0].rules[0].matchers.header_equals;
        assert_eq!(headers.get("X-Flarion-Route"), Some(&"fast".to_string()));
    }

    fn valid_route(id: &str, target: &str) -> RouteConfig {
        RouteConfig {
            id: id.into(),
            first_token_timeout_ms: None,
            rules: vec![RuleConfig {
                name: "default".into(),
                matchers: Matchers::default(),
                targets: vec![target.into()],
                first_token_timeout_ms: None,
            }],
        }
    }

    #[test]
    fn test_validate_accepts_config_with_valid_route() {
        let tmp = std::env::temp_dir().join("flarion-route-ok.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![valid_route("chat", "m1")],
            ..FlarionConfig::default()
        };
        assert!(config.validate().is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_empty_route_id() {
        let tmp = std::env::temp_dir().join("flarion-route-empty.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut route = valid_route("placeholder", "m1");
        route.id = "".into();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![route],
            ..FlarionConfig::default()
        };
        assert!(matches!(config.validate(), Err(ConfigError::RouteEmptyId)));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_duplicate_route_id() {
        let tmp = std::env::temp_dir().join("flarion-route-dup.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![valid_route("chat", "m1"), valid_route("chat", "m1")],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateRouteId { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_route_id_collision_with_model() {
        let tmp = std::env::temp_dir().join("flarion-route-coll.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("chat", tmp.clone())],
            routes: vec![valid_route("chat", "chat")],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::RouteIdCollision { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_route_with_no_rules() {
        let tmp = std::env::temp_dir().join("flarion-route-norules.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![RouteConfig {
                id: "chat".into(),
                first_token_timeout_ms: None,
                rules: Vec::new(),
            }],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::RouteNoRules { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_rule_with_no_targets() {
        let tmp = std::env::temp_dir().join("flarion-route-notargets.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![RouteConfig {
                id: "chat".into(),
                first_token_timeout_ms: None,
                rules: vec![RuleConfig {
                    name: "default".into(),
                    matchers: Matchers::default(),
                    targets: Vec::new(),
                    first_token_timeout_ms: None,
                }],
            }],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::RuleNoTargets { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_unknown_target() {
        let tmp = std::env::temp_dir().join("flarion-route-unknown.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![valid_route("chat", "nonexistent")],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::RouteTargetUnknown { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_route_targeting_route() {
        let tmp = std::env::temp_dir().join("flarion-route-ror.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![valid_route("chat", "m1"), valid_route("chat2", "chat")],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::RouteTargetsRoute { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_route_without_catch_all() {
        let tmp = std::env::temp_dir().join("flarion-route-nocatch.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![RouteConfig {
                id: "chat".into(),
                first_token_timeout_ms: None,
                rules: vec![RuleConfig {
                    name: "r1".into(),
                    matchers: Matchers {
                        stream: Some(true),
                        ..Matchers::default()
                    },
                    targets: vec!["m1".into()],
                    first_token_timeout_ms: None,
                }],
            }],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::RouteNoCatchAll { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_invalid_regex() {
        let tmp = std::env::temp_dir().join("flarion-route-badregex.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            models: vec![local_cfg("m1", tmp.clone())],
            routes: vec![RouteConfig {
                id: "chat".into(),
                first_token_timeout_ms: None,
                rules: vec![
                    RuleConfig {
                        name: "bad".into(),
                        matchers: Matchers {
                            content_regex: Some("(unclosed".into()),
                            ..Matchers::default()
                        },
                        targets: vec!["m1".into()],
                        first_token_timeout_ms: None,
                    },
                    RuleConfig {
                        name: "catch".into(),
                        matchers: Matchers::default(),
                        targets: vec!["m1".into()],
                        first_token_timeout_ms: None,
                    },
                ],
            }],
            ..FlarionConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidRegex { .. })
        ));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_binds_loopback_ipv4() {
        let s = ServerConfig {
            host: "127.0.0.1".into(),
            ..ServerConfig::default()
        };
        assert!(s.binds_loopback());
    }

    #[test]
    fn test_binds_loopback_ipv4_whole_loopback_block() {
        let s = ServerConfig {
            host: "127.1.2.3".into(),
            ..ServerConfig::default()
        };
        assert!(s.binds_loopback());
    }

    #[test]
    fn test_binds_loopback_ipv6() {
        let s = ServerConfig {
            host: "::1".into(),
            ..ServerConfig::default()
        };
        assert!(s.binds_loopback());
    }

    #[test]
    fn test_binds_loopback_hostname() {
        let s = ServerConfig {
            host: "localhost".into(),
            ..ServerConfig::default()
        };
        assert!(s.binds_loopback());
        let s = ServerConfig {
            host: "LOCALHOST".into(),
            ..ServerConfig::default()
        };
        assert!(s.binds_loopback());
    }

    #[test]
    fn test_binds_loopback_public_bind_false() {
        for host in ["0.0.0.0", "::", "192.168.1.1", "10.0.0.1", "example.com"] {
            let s = ServerConfig {
                host: host.into(),
                ..ServerConfig::default()
            };
            assert!(!s.binds_loopback(), "host {host} should NOT be loopback");
        }
    }

    #[test]
    fn test_binds_loopback_empty_or_garbage() {
        for host in ["", "not-an-ip-or-hostname-[[[", "  "] {
            let s = ServerConfig {
                host: host.into(),
                ..ServerConfig::default()
            };
            assert!(
                !s.binds_loopback(),
                "host '{host}' should NOT be loopback (can't parse → public assumed)"
            );
        }
    }

    #[test]
    fn test_parse_server_new_phase2d_fields() {
        let toml_str = r#"
[server]
api_keys = ["k1"]
allow_unauthenticated = true
cors_origins = ["https://app.example.com"]
allow_plaintext_upstream = true

[[models]]
id = "m"
backend = "local"
path = "/tmp/m.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert!(config.server.allow_unauthenticated);
        assert_eq!(
            config.server.cors_origins,
            vec!["https://app.example.com".to_string()]
        );
        assert!(config.server.allow_plaintext_upstream);
    }

    #[test]
    fn test_parse_server_new_fields_defaults() {
        let toml_str = r#"
[server]

[[models]]
id = "m"
backend = "local"
path = "/tmp/m.gguf"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.server.allow_unauthenticated);
        assert!(config.server.cors_origins.is_empty());
        assert!(!config.server.allow_plaintext_upstream);
    }

    #[test]
    fn test_ssrf_accepts_https_public() {
        assert!(validate_upstream_url("m", "https://api.openai.com/v1", false).is_ok());
        assert!(validate_upstream_url("m", "https://api.example.com", false).is_ok());
    }

    #[test]
    fn test_ssrf_rejects_http_without_optin() {
        let err = validate_upstream_url("m", "http://api.example.com", false).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PlaintextUpstreamForbidden { .. }
        ));
    }

    #[test]
    fn test_ssrf_accepts_http_with_optin() {
        assert!(validate_upstream_url("m", "http://api.example.com", true).is_ok());
    }

    #[test]
    fn test_ssrf_rejects_loopback_v4_without_optin() {
        for url in ["https://127.0.0.1/v1", "https://127.1.2.3/v1"] {
            let err = validate_upstream_url("m", url, false).unwrap_err();
            assert!(
                matches!(err, ConfigError::PlaintextUpstreamForbidden { .. }),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn test_ssrf_rejects_loopback_v6() {
        let err = validate_upstream_url("m", "https://[::1]/v1", false).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PlaintextUpstreamForbidden { .. }
        ));
    }

    #[test]
    fn test_ssrf_rejects_link_local_v4() {
        let err = validate_upstream_url("m", "https://169.254.169.254/v1", false).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PlaintextUpstreamForbidden { .. }
        ));
    }

    #[test]
    fn test_ssrf_rejects_private_range_v4() {
        for url in [
            "https://10.0.0.1/v1",
            "https://172.16.0.1/v1",
            "https://192.168.1.1/v1",
        ] {
            let err = validate_upstream_url("m", url, false).unwrap_err();
            assert!(
                matches!(err, ConfigError::PlaintextUpstreamForbidden { .. }),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn test_ssrf_rejects_localhost_domain() {
        let err = validate_upstream_url("m", "https://localhost/v1", false).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PlaintextUpstreamForbidden { .. }
        ));
        let err = validate_upstream_url("m", "https://LOCALHOST/v1", false).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PlaintextUpstreamForbidden { .. }
        ));
    }

    #[test]
    fn test_ssrf_rejects_unsupported_scheme() {
        let err = validate_upstream_url("m", "file:///etc/passwd", false).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidBaseUrl { .. }));
    }

    #[test]
    fn test_ssrf_rejects_malformed_url() {
        let err = validate_upstream_url("m", "not a url at all", false).unwrap_err();
        assert!(matches!(err, ConfigError::InvalidBaseUrl { .. }));
    }

    #[test]
    fn test_ssrf_accepts_private_with_optin() {
        for url in [
            "https://127.0.0.1/v1",
            "http://10.0.0.1/v1",
            "https://localhost/v1",
        ] {
            assert!(
                validate_upstream_url("m", url, true).is_ok(),
                "{url} should be accepted with opt-in"
            );
        }
    }

    #[test]
    fn test_validate_rejects_cloud_with_http_base_url() {
        let mut m = cloud_cfg("gpt-4o", BackendType::Openai, Some("sk"));
        m.base_url = Some("http://api.example.com/v1".into());
        let mut config = FlarionConfig {
            models: vec![m],
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(
            err,
            ConfigError::PlaintextUpstreamForbidden { .. }
        ));
    }

    #[test]
    fn test_validate_accepts_cloud_with_http_and_optin() {
        let mut m = cloud_cfg("gpt-4o", BackendType::Openai, Some("sk"));
        m.base_url = Some("http://api.example.com/v1".into());
        let mut config = FlarionConfig {
            server: ServerConfig {
                allow_plaintext_upstream: true,
                ..ServerConfig::default()
            },
            models: vec![m],
            ..FlarionConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_parse_metrics_bind() {
        let toml_str = r#"
[server]

[[models]]
id = "m"
backend = "local"
path = "/tmp/m.gguf"

[metrics]
enabled = true
bind = "127.0.0.1:9091"
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.metrics.bind.as_deref(), Some("127.0.0.1:9091"));
    }

    #[test]
    fn test_parse_model_max_tokens_cap() {
        let toml_str = r#"
[server]

[[models]]
id = "m"
backend = "local"
path = "/tmp/m.gguf"
max_tokens_cap = 16384
"#;
        let config: FlarionConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.models[0].max_tokens_cap, Some(16384));
    }

    #[test]
    fn test_validate_rejects_invalid_metrics_bind() {
        let tmp = std::env::temp_dir().join("flarion-mbind.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            metrics: MetricsConfig {
                enabled: true,
                bind: Some("not-a-socket-addr".into()),
                ..MetricsConfig::default()
            },
            models: vec![local_cfg("m", tmp.clone())],
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidMetricsBind { .. }));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_accepts_valid_metrics_bind() {
        let tmp = std::env::temp_dir().join("flarion-mbind-ok.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            metrics: MetricsConfig {
                enabled: true,
                bind: Some("127.0.0.1:9091".into()),
                ..MetricsConfig::default()
            },
            models: vec![local_cfg("m", tmp.clone())],
            ..FlarionConfig::default()
        };
        assert!(config.validate().is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_rejects_invalid_cors_origin() {
        let tmp = std::env::temp_dir().join("flarion-cors.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            server: ServerConfig {
                cors_origins: vec!["not a url".into()],
                ..ServerConfig::default()
            },
            models: vec![local_cfg("m", tmp.clone())],
            ..FlarionConfig::default()
        };
        let err = config.validate().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidCorsOrigin { .. }));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_accepts_valid_cors_origin() {
        let tmp = std::env::temp_dir().join("flarion-cors-ok.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut config = FlarionConfig {
            server: ServerConfig {
                cors_origins: vec!["https://app.example.com".into()],
                ..ServerConfig::default()
            },
            models: vec![local_cfg("m", tmp.clone())],
            ..FlarionConfig::default()
        };
        assert!(config.validate().is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_server_config_default_shutdown_grace_secs_is_30() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.shutdown_grace_secs, 30);
    }

    #[test]
    fn test_toml_omits_shutdown_grace_secs_uses_default() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 8080

[[models]]
id = "m"
backend = "local"
path = "/tmp/x.gguf"
context_size = 4096
gpu_layers = 99
"#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.shutdown_grace_secs, 30);
    }

    #[test]
    fn test_toml_shutdown_grace_secs_parsed() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 8080
shutdown_grace_secs = 60

[[models]]
id = "m"
backend = "local"
path = "/tmp/x.gguf"
context_size = 4096
gpu_layers = 99
"#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.shutdown_grace_secs, 60);
    }

    #[test]
    fn test_validate_clamps_shutdown_grace_secs_over_limit() {
        let tmp = std::env::temp_dir().join("flarion-shutdown-clamp.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut cfg = FlarionConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                shutdown_grace_secs: 99999,
                ..ServerConfig::default()
            },
            models: vec![local_cfg("m", tmp.clone())],
            ..FlarionConfig::default()
        };
        cfg.validate().unwrap();
        assert_eq!(cfg.server.shutdown_grace_secs, 3600);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_validate_shutdown_grace_secs_exact_max_preserved() {
        let tmp = std::env::temp_dir().join("flarion-shutdown-exact.gguf");
        std::fs::write(&tmp, b"").unwrap();
        let mut cfg = FlarionConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                shutdown_grace_secs: 3600,
                ..ServerConfig::default()
            },
            models: vec![local_cfg("m", tmp.clone())],
            ..FlarionConfig::default()
        };
        cfg.validate().unwrap();
        assert_eq!(cfg.server.shutdown_grace_secs, 3600);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_server_config_default_vram_budget_is_0() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.vram_budget_mb, VramBudgetSetting::Fixed(0));
    }

    #[test]
    fn test_toml_omits_vram_budget_uses_default() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 8080

[[models]]
id = "m"
backend = "local"
path = "/tmp/x.gguf"
context_size = 4096
gpu_layers = 99
"#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.vram_budget_mb, VramBudgetSetting::Fixed(0));
        assert!(!cfg.models[0].lazy);
        assert!(cfg.models[0].vram_mb.is_none());
    }

    #[test]
    fn test_toml_parses_vram_budget_lazy_vram_mb() {
        let toml = r#"
[server]
host = "127.0.0.1"
port = 8080
vram_budget_mb = 22000

[[models]]
id = "m"
backend = "local"
path = "/tmp/x.gguf"
context_size = 4096
gpu_layers = 99
lazy = true
vram_mb = 6000
"#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.vram_budget_mb, VramBudgetSetting::Fixed(22000));
        assert!(cfg.models[0].lazy);
        assert_eq!(cfg.models[0].vram_mb, Some(6000));
    }

    // Helper: sparse tempfile of `size_mb` MB. Returns (TempDir, path).
    fn make_fake_gguf(size_mb: u64) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("fake-{size_mb}.gguf"));
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(size_mb * 1024 * 1024).unwrap();
        drop(f);
        (dir, path)
    }

    #[test]
    fn test_validate_rejects_lazy_on_cloud_backend() {
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.models = vec![ModelConfig {
            id: "cloud".into(),
            backend: BackendType::Openai,
            path: None,
            context_size: 4096,
            gpu_layers: 0,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: Some("k".into()),
            base_url: Some("https://api.openai.com/v1".into()),
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: true,
            vram_mb: None,
            pin: false,
            gpus: vec![],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }];
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::LazyOnlyForLocal { .. }));
    }

    #[test]
    fn test_validate_rejects_vram_mb_on_cloud_backend() {
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.models = vec![ModelConfig {
            id: "cloud".into(),
            backend: BackendType::Openai,
            path: None,
            context_size: 4096,
            gpu_layers: 0,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: Some("k".into()),
            base_url: Some("https://api.openai.com/v1".into()),
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: Some(1000),
            pin: false,
            gpus: vec![],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }];
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, ConfigError::VramMbOnlyForLocal { .. }));
    }

    #[test]
    fn test_validate_accepts_eager_under_budget() {
        let (_a_dir, a_path) = make_fake_gguf(100);
        let (_b_dir, b_path) = make_fake_gguf(100);
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(500);
        let mut a = local_cfg("a", a_path);
        a.gpus = vec![0];
        let mut b = local_cfg("b", b_path);
        b.gpus = vec![0];
        cfg.models = vec![a, b];
        // 100MB * 1.2 = 120MB each → 240MB total < 500MB budget.
        cfg.validate().unwrap();
    }

    #[test]
    fn test_validate_rejects_eager_budget_overflow() {
        let (_a_dir, a_path) = make_fake_gguf(200);
        let (_b_dir, b_path) = make_fake_gguf(200);
        let (_c_dir, c_path) = make_fake_gguf(200);
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(500);
        // Pin all three models to gpu 0 so per-device check applies.
        let mut a = local_cfg("a", a_path);
        a.gpus = vec![0];
        let mut b = local_cfg("b", b_path);
        b.gpus = vec![0];
        let mut c = local_cfg("c", c_path);
        c.gpus = vec![0];
        cfg.models = vec![a, b, c];
        // Each ≈240MB, total ≈720MB > 500MB on gpu 0.
        let err = cfg.validate().unwrap_err();
        match err {
            ConfigError::EagerLoadsExceedBudget {
                gpu_id,
                total_mb,
                budget_mb,
                offenders,
            } => {
                assert_eq!(gpu_id, 0);
                assert!(total_mb > 500, "total_mb={total_mb}");
                assert_eq!(budget_mb, 500);
                assert_eq!(offenders.len(), 3);
                let ids: std::collections::HashSet<_> =
                    offenders.iter().map(|(id, _)| id.as_str()).collect();
                assert!(ids.contains("a"));
                assert!(ids.contains("b"));
                assert!(ids.contains("c"));
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn test_validate_lazy_excluded_from_eager_sum() {
        let (_a_dir, a_path) = make_fake_gguf(200);
        let (_b_dir, b_path) = make_fake_gguf(200);
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(300);
        let mut eager_model = local_cfg("eager", a_path);
        eager_model.gpus = vec![0];
        let mut lazy_model = local_cfg("lazy", b_path);
        lazy_model.lazy = true;
        // lazy_model.gpus stays vec![] (Auto) — correctly skipped by per-device check.
        cfg.models = vec![eager_model, lazy_model];
        // Eager: 240MB < 300MB budget. Lazy excluded from sum.
        cfg.validate().unwrap();
    }

    #[test]
    fn test_pin_deserializes_default_false() {
        let toml = r#"
        [server]
        host = "127.0.0.1"
        [[models]]
        id = "m"
        backend = "local"
        path = "/tmp/m.gguf"
    "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.models[0].pin);
    }

    #[test]
    fn test_pin_deserializes_true() {
        let toml = r#"
        [server]
        host = "127.0.0.1"
        [[models]]
        id = "m"
        backend = "local"
        path = "/tmp/m.gguf"
        pin = true
    "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert!(cfg.models[0].pin);
    }

    #[test]
    fn test_validate_rejects_pin_on_cloud_backend() {
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.models = vec![ModelConfig {
            id: "openai-m".into(),
            backend: BackendType::Openai,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: Some("k".into()),
            base_url: Some("https://api.openai.com/v1".into()),
            upstream_model: Some("gpt-4o".into()),
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: true,
            gpus: vec![],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }];
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::PinOnlyForLocal { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn test_validate_rejects_pinned_over_budget() {
        // Each file is 200 MB → estimate ~240 MB. Three pinned models on gpu 0 = ~720 MB.
        let (_a_dir, a_path) = make_fake_gguf(200);
        let (_b_dir, b_path) = make_fake_gguf(200);
        let (_c_dir, c_path) = make_fake_gguf(200);

        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(500);
        // Pin all three to gpu 0 so per-device check applies.
        cfg.models = vec![
            ModelConfig {
                id: "a".into(),
                backend: BackendType::Local,
                path: Some(a_path),
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: true,
                vram_mb: None,
                pin: true,
                gpus: vec![0],
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
            },
            ModelConfig {
                id: "b".into(),
                backend: BackendType::Local,
                path: Some(b_path),
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: true,
                vram_mb: None,
                pin: true,
                gpus: vec![0],
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
            },
            ModelConfig {
                id: "c".into(),
                backend: BackendType::Local,
                path: Some(c_path),
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: true,
                vram_mb: None,
                pin: true,
                gpus: vec![0],
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
            },
        ];

        let err = cfg.validate().unwrap_err();
        match err {
            ConfigError::PinnedExceedsBudget { gpu_id, total_mb, budget_mb, offenders } => {
                assert_eq!(gpu_id, 0);
                assert!(total_mb > budget_mb, "total={total_mb} budget={budget_mb}");
                assert_eq!(budget_mb, 500);
                assert_eq!(offenders.len(), 3);
                let ids: Vec<&str> = offenders.iter().map(|(id, _)| id.as_str()).collect();
                assert!(ids.contains(&"a"));
                assert!(ids.contains(&"b"));
                assert!(ids.contains(&"c"));
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn test_vram_budget_setting_deserializes_fixed_integer() {
        let toml = r#"
        [server]
        host = "127.0.0.1"
        vram_budget_mb = 22000
        [[models]]
        id = "m"
        backend = "local"
        path = "/tmp/m.gguf"
    "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.vram_budget_mb, VramBudgetSetting::Fixed(22000));
    }

    #[test]
    fn test_vram_budget_setting_deserializes_auto_string() {
        let toml = r#"
        [server]
        host = "127.0.0.1"
        vram_budget_mb = "auto"
        [[models]]
        id = "m"
        backend = "local"
        path = "/tmp/m.gguf"
    "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.vram_budget_mb, VramBudgetSetting::Auto);
    }

    #[test]
    fn test_vram_budget_setting_rejects_other_strings() {
        let toml = r#"
        [server]
        host = "127.0.0.1"
        vram_budget_mb = "not-auto"
        [[models]]
        id = "m"
        backend = "local"
        path = "/tmp/m.gguf"
    "#;
        let err = toml::from_str::<FlarionConfig>(toml).unwrap_err();
        assert!(format!("{err}").contains("auto"), "got {err}");
    }

    #[test]
    fn test_vram_budget_setting_defaults_to_fixed_zero() {
        let toml = r#"
        [server]
        host = "127.0.0.1"
        [[models]]
        id = "m"
        backend = "local"
        path = "/tmp/m.gguf"
    "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.vram_budget_mb, VramBudgetSetting::Fixed(0));
    }

    #[test]
    fn test_resolve_fixed_passes_through() {
        let server = ServerConfig { vram_budget_mb: VramBudgetSetting::Fixed(22000), ..ServerConfig::default() };
        assert_eq!(server.resolve_vram_budget_mb().unwrap(), 22000);
    }

    #[test]
    fn test_resolve_fixed_zero_is_disabled() {
        let server = ServerConfig::default();
        assert_eq!(server.resolve_vram_budget_mb().unwrap(), 0);
    }

    #[test]
    fn test_resolve_headroom_exceeds_total_returns_insufficient() {
        use crate::engine::vram_detect::VramInfo;
        let info = VramInfo { device_index: 0, total_mb: 1000, free_mb: 500 };
        let err = ServerConfig::resolve_vram_budget_mb_from_info(&info, 2000).unwrap_err();
        assert!(
            matches!(err, ConfigError::VramAutoDetectInsufficient { total_mb: 1000, headroom_mb: 2000 }),
            "got {err:?}"
        );
    }

    #[test]
    fn test_resolve_subtracts_headroom_from_total() {
        use crate::engine::vram_detect::VramInfo;
        let info = VramInfo { device_index: 0, total_mb: 24000, free_mb: 20000 };
        let got = ServerConfig::resolve_vram_budget_mb_from_info(&info, 2000).unwrap();
        assert_eq!(got, 22000);
    }

    #[test]
    fn test_gpus_deserializes_default_empty() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            [[models]]
            id = "m"
            backend = "local"
            path = "/tmp/m.gguf"
        "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert!(cfg.models[0].gpus.is_empty());
    }

    #[test]
    fn test_gpus_deserializes_single_device() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            [[models]]
            id = "m"
            backend = "local"
            path = "/tmp/m.gguf"
            gpus = [2]
        "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.models[0].gpus, vec![2]);
    }

    #[test]
    fn test_gpus_deserializes_tensor_split_list() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            [[models]]
            id = "m"
            backend = "local"
            path = "/tmp/m.gguf"
            gpus = [0, 1, 2]
        "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.models[0].gpus, vec![0, 1, 2]);
    }

    #[test]
    fn test_validate_rejects_duplicate_gpu_in_gpus() {
        let dir = tempfile::tempdir().unwrap();
        let path = {
            let p = dir.path().join("m.gguf");
            let f = std::fs::File::create(&p).unwrap();
            f.set_len(100 * 1024 * 1024).unwrap();
            drop(f);
            p
        };
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.models = vec![ModelConfig {
            id: "m".into(),
            backend: BackendType::Local,
            path: Some(path),
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: None,
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
            gpus: vec![0, 1, 0],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }];
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::GpuIdDuplicated { gpu_id: 0, .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn test_validate_rejects_gpus_on_cloud_backend() {
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.models = vec![ModelConfig {
            id: "openai-m".into(),
            backend: BackendType::Openai,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: Some("k".into()),
            base_url: Some("https://api.openai.com/v1".into()),
            upstream_model: Some("gpt-4o".into()),
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
            gpus: vec![0],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }];
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::GpuIdOnCloudBackend { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn test_vram_budget_overrides_deserializes() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            vram_budget_overrides = { 0 = 20000, 1 = 24000 }
            [[models]]
            id = "m"
            backend = "local"
            path = "/tmp/m.gguf"
        "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.server.vram_budget_overrides.get(&0), Some(&20000));
        assert_eq!(cfg.server.vram_budget_overrides.get(&1), Some(&24000));
    }

    #[test]
    fn test_vram_budget_overrides_defaults_to_empty() {
        let toml = r#"
            [server]
            host = "127.0.0.1"
            [[models]]
            id = "m"
            backend = "local"
            path = "/tmp/m.gguf"
        "#;
        let cfg: FlarionConfig = toml::from_str(toml).unwrap();
        assert!(cfg.server.vram_budget_overrides.is_empty());
    }

    #[test]
    fn test_resolve_budgets_fixed_mode_uniform() {
        let server = ServerConfig {
            vram_budget_mb: VramBudgetSetting::Fixed(22000),
            ..ServerConfig::default()
        };
        let budgets = server.resolve_vram_budgets(2).unwrap();
        assert_eq!(budgets, vec![22000, 22000]);
    }

    #[test]
    fn test_resolve_budgets_fixed_mode_single_device_default() {
        let server = ServerConfig {
            vram_budget_mb: VramBudgetSetting::Fixed(22000),
            ..ServerConfig::default()
        };
        let budgets = server.resolve_vram_budgets(1).unwrap();
        assert_eq!(budgets, vec![22000]);
    }

    #[test]
    fn test_resolve_budgets_override_wins() {
        let server = ServerConfig {
            vram_budget_mb: VramBudgetSetting::Fixed(22000),
            vram_budget_overrides: [(0u32, 20000u64)].into_iter().collect(),
            ..ServerConfig::default()
        };
        let budgets = server.resolve_vram_budgets(2).unwrap();
        assert_eq!(budgets, vec![20000, 22000]);
    }

    #[test]
    fn test_resolve_budgets_rejects_override_unknown_gpu() {
        let server = ServerConfig {
            vram_budget_mb: VramBudgetSetting::Fixed(22000),
            vram_budget_overrides: [(3u32, 10000u64)].into_iter().collect(),
            ..ServerConfig::default()
        };
        let err = server.resolve_vram_budgets(2).unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::VramOverrideUnknownGpu { gpu_id: 3, device_count: 2 }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn test_resolve_budgets_fixed_zero_passes_through_per_device() {
        let server = ServerConfig::default();
        let budgets = server.resolve_vram_budgets(2).unwrap();
        assert_eq!(budgets, vec![0, 0]);
    }

    #[test]
    fn test_validate_rejects_eager_overflow_on_single_gpu_out_of_many() {
        let dir = tempfile::tempdir().unwrap();
        // 400 MB file → estimate 480 MB. Three of them → ~1440 MB on gpu 0.
        let mk = |name: &str| {
            let p = dir.path().join(name);
            let f = std::fs::File::create(&p).unwrap();
            f.set_len(400 * 1024 * 1024).unwrap();
            drop(f);
            p
        };
        fn eager_on_gpu(id: &str, path: std::path::PathBuf, gpu: u32) -> ModelConfig {
            ModelConfig {
                id: id.into(),
                backend: BackendType::Local,
                path: Some(path),
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: false,
                vram_mb: None,
                pin: false,
                gpus: vec![gpu],
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
            }
        }

        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(1000);
        cfg.models = vec![
            eager_on_gpu("a", mk("a.gguf"), 0),
            eager_on_gpu("b", mk("b.gguf"), 0),
            eager_on_gpu("c", mk("c.gguf"), 0),
        ];
        let err = cfg.validate().unwrap_err();
        match err {
            ConfigError::EagerLoadsExceedBudget {
                gpu_id,
                total_mb,
                budget_mb,
                ..
            } => {
                assert_eq!(gpu_id, 0);
                assert!(total_mb > budget_mb);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn test_validate_rejects_pinned_overflow_with_split_model() {
        let dir = tempfile::tempdir().unwrap();
        let mk = |name: &str, mb_on_disk: u64| {
            let p = dir.path().join(name);
            let f = std::fs::File::create(&p).unwrap();
            f.set_len(mb_on_disk * 1024 * 1024).unwrap();
            drop(f);
            p
        };
        fn pinned_lazy(id: &str, path: std::path::PathBuf, gpus: Vec<u32>) -> ModelConfig {
            ModelConfig {
                id: id.into(),
                backend: BackendType::Local,
                path: Some(path),
                context_size: 4096,
                gpu_layers: 99,
                threads: None,
                batch_size: None,
                seed: None,
                api_key: None,
                base_url: None,
                upstream_model: None,
                timeout_secs: None,
                max_tokens_cap: None,
                lazy: true,
                vram_mb: None,
                pin: true,
                gpus,
                repo: None,
                revision: None,
                dtype: None,
                hf_token_env: None,
                adapters: Vec::new(),
            }
        }

        // Split model 10000 MB across gpus [0, 1] → 5000 MB each.
        // Single-device pinned 6000 MB on gpu 0.
        // Total on gpu 0: 11000 MB. Budget: 10000 → overflow on gpu 0.
        let mut cfg = FlarionConfig::default();
        cfg.server.host = "127.0.0.1".into();
        cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(10000);
        let mut split = pinned_lazy("split", mk("split.gguf", 100), vec![0, 1]);
        split.vram_mb = Some(10000);
        let mut single = pinned_lazy("single", mk("single.gguf", 100), vec![0]);
        single.vram_mb = Some(6000);
        cfg.models = vec![split, single];

        let err = cfg.validate().unwrap_err();
        match err {
            ConfigError::PinnedExceedsBudget {
                gpu_id, total_mb, budget_mb, ..
            } => {
                assert_eq!(gpu_id, 0);
                assert_eq!(total_mb, 11000);
                assert_eq!(budget_mb, 10000);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn test_backend_type_parses_hf() {
        let toml = r#"backend = "hf""#;
        #[derive(serde::Deserialize)]
        struct Wrap { backend: BackendType }
        let parsed: Wrap = toml::from_str(toml).unwrap();
        assert_eq!(parsed.backend, BackendType::Hf);
    }

    #[test]
    fn test_dtype_parses_each_variant() {
        #[derive(serde::Deserialize)]
        struct Wrap { dtype: Dtype }
        for (literal, expected) in [
            (r#"dtype = "bf16""#, Dtype::Bf16),
            (r#"dtype = "fp16""#, Dtype::Fp16),
            (r#"dtype = "q4_0""#, Dtype::Q4_0),
            (r#"dtype = "q4_k_m""#, Dtype::Q4KM),
            (r#"dtype = "q8_0""#, Dtype::Q8_0),
        ] {
            let parsed: Wrap = toml::from_str(literal).unwrap();
            assert_eq!(parsed.dtype, expected);
        }
    }

    #[test]
    fn test_dtype_rejects_unknown_variant() {
        #[derive(Debug, serde::Deserialize)]
        struct Wrap { dtype: Dtype }
        let toml = r#"dtype = "fp32""#;
        let err = toml::from_str::<Wrap>(toml).unwrap_err();
        assert!(err.to_string().contains("fp32"), "unexpected error: {err}");
    }

    #[test]
    fn test_adapter_config_parses_minimal_local() {
        let toml = r#"
path = "/adapters/my-lora"
"#;
        let a: AdapterConfig = toml::from_str(toml).unwrap();
        assert_eq!(a.path.as_deref(), Some(std::path::Path::new("/adapters/my-lora")));
        assert_eq!(a.repo, None);
        assert_eq!(a.revision, None);
        assert_eq!(a.scale, 1.0);
    }

    #[test]
    fn test_adapter_config_parses_hub_with_scale() {
        let toml = r#"
repo = "org/my-adapter"
revision = "v1"
scale = 0.5
"#;
        let a: AdapterConfig = toml::from_str(toml).unwrap();
        assert_eq!(a.path, None);
        assert_eq!(a.repo.as_deref(), Some("org/my-adapter"));
        assert_eq!(a.revision.as_deref(), Some("v1"));
        assert_eq!(a.scale, 0.5);
    }

    #[test]
    fn test_model_config_parses_hf_fields() {
        let toml = r#"
id = "m"
backend = "hf"
repo = "org/model"
revision = "main"
dtype = "bf16"
hf_token_env = "HF_TOKEN"

[[adapters]]
path = "/adapters/a"

[[adapters]]
repo = "org/b"
scale = 0.3
"#;
        let m: ModelConfig = toml::from_str(toml).unwrap();
        assert_eq!(m.id, "m");
        assert_eq!(m.backend, BackendType::Hf);
        assert_eq!(m.repo.as_deref(), Some("org/model"));
        assert_eq!(m.revision.as_deref(), Some("main"));
        assert_eq!(m.dtype, Some(Dtype::Bf16));
        assert_eq!(m.hf_token_env.as_deref(), Some("HF_TOKEN"));
        assert_eq!(m.adapters.len(), 2);
        assert_eq!(m.adapters[0].path.as_deref(), Some(std::path::Path::new("/adapters/a")));
        assert_eq!(m.adapters[1].repo.as_deref(), Some("org/b"));
        assert_eq!(m.adapters[1].scale, 0.3);
    }

    #[test]
    fn test_model_config_hf_fields_default_none_on_non_hf() {
        let toml = r#"
id = "m"
backend = "local"
path = "/tmp/m.gguf"
"#;
        let m: ModelConfig = toml::from_str(toml).unwrap();
        assert_eq!(m.repo, None);
        assert_eq!(m.revision, None);
        assert_eq!(m.dtype, None);
        assert_eq!(m.hf_token_env, None);
        assert!(m.adapters.is_empty());
    }

    #[test]
    fn test_config_error_variants_exist_for_hf() {
        // Type-system test: these variants must be constructible with these shapes.
        let _ = ConfigError::HfBackendNeedsPathOrRepo { id: "m".into() };
        let _ = ConfigError::HfBackendPathAndRepoExclusive { id: "m".into() };
        let _ = ConfigError::HfFieldOnNonHfBackend {
            id: "m".into(),
            field: "dtype".into(),
        };
        let _ = ConfigError::HfAdapterNeedsPathOrRepo {
            id: "m".into(),
            index: 0,
        };
        let _ = ConfigError::HfAdapterPathAndRepoExclusive {
            id: "m".into(),
            index: 0,
        };
    }

    fn hf_cfg_with(id: &str, apply: impl FnOnce(&mut ModelConfig)) -> ModelConfig {
        let mut m = ModelConfig {
            id: id.into(),
            backend: BackendType::Hf,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: None,
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
            gpus: Vec::new(),
            repo: Some("org/model".into()),
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        };
        apply(&mut m);
        m
    }

    fn config_with_single_model(m: ModelConfig) -> FlarionConfig {
        FlarionConfig {
            server: ServerConfig::default(),
            logging: LoggingConfig::default(),
            metrics: crate::config::MetricsConfig::default(),
            models: vec![m],
            routes: Vec::new(),
        }
    }

    #[test]
    fn test_hf_config_needs_path_or_repo() {
        let m = hf_cfg_with("m", |m| {
            m.repo = None;
            m.path = None;
        });
        let err = config_with_single_model(m).validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::HfBackendNeedsPathOrRepo { ref id } if id == "m"),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_hf_config_rejects_both_path_and_repo() {
        let m = hf_cfg_with("m", |m| {
            m.repo = Some("org/x".into());
            m.path = Some("/tmp/x".into());
        });
        let err = config_with_single_model(m).validate().unwrap_err();
        assert!(
            matches!(err, ConfigError::HfBackendPathAndRepoExclusive { ref id } if id == "m"),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_local_backend_rejects_hf_fields() {
        // Use a real tempfile so ModelPathMissing can't short-circuit — this
        // exercises reject_hf_fields_on_non_hf() end to end.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut m = local_cfg("m", tmp.path().to_path_buf());
        m.dtype = Some(Dtype::Bf16);
        let err = config_with_single_model(m).validate().unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::HfFieldOnNonHfBackend { ref id, ref field }
                    if id == "m" && field == "dtype"
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_hf_adapter_needs_path_or_repo() {
        let m = hf_cfg_with("m", |m| {
            m.adapters = vec![AdapterConfig {
                path: None,
                repo: None,
                revision: None,
                scale: 1.0,
            }];
        });
        let err = config_with_single_model(m).validate().unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::HfAdapterNeedsPathOrRepo { ref id, index: 0 } if id == "m"
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn test_hf_adapter_rejects_both_path_and_repo() {
        let m = hf_cfg_with("m", |m| {
            m.adapters = vec![AdapterConfig {
                path: Some("/a".into()),
                repo: Some("org/a".into()),
                revision: None,
                scale: 1.0,
            }];
        });
        let err = config_with_single_model(m).validate().unwrap_err();
        assert!(
            matches!(
                err,
                ConfigError::HfAdapterPathAndRepoExclusive { ref id, index: 0 } if id == "m"
            ),
            "got: {err:?}"
        );
    }
}
