use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildInfo {
    pub version: String,
    pub git_sha: Option<String>,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServerInfo {
    pub version: String,
    pub git_sha: Option<String>,
    pub uptime_s: u64,
    pub bind: String,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Gpu {
    pub id: u32,
    pub name: String,
    pub budget_mb: u64,
    pub reserved_mb: u64,
    pub free_mb: u64,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Model {
    pub id: String,
    pub backend: String,
    pub state: String,
    pub pinned: bool,
    pub lazy: bool,
    pub vram_mb: Option<u64>,
    pub gpus: Vec<u32>,
    pub in_flight: u64,
    pub last_used_s: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Route {
    pub id: String,
    pub rules: Vec<RouteRule>,
    pub fallback_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteRule {
    pub name: String,
    pub hit_count: u64,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentRollup {
    pub requests_last_60s: u64,
    pub errors_last_60s: u64,
    pub ttft_p50_ms: Option<u64>,
    pub ttft_p95_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub server: ServerInfo,
    pub gpus: Vec<Gpu>,
    pub models: Vec<Model>,
    pub in_flight_total: u64,
    pub recent: RecentRollup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RequestEvent {
    Started { id: String, ts: String, route: Option<String>, backend: String },
    FirstToken { id: String, ts: String, ttft_ms: u64 },
    Completed {
        id: String, ts: String, route: Option<String>, matched_rule: Option<String>,
        backend: String, fallback_count: u32, status: String,
        ttft_ms: Option<u64>, duration_ms: u64,
        prompt_tokens: u64, completion_tokens: u64,
    },
    Failed { id: String, ts: String, backend: String, reason: String, duration_ms: u64 },
    Canceled { id: String, ts: String, backend: String, duration_ms: u64 },
    Gap { missed: u64 },
}
