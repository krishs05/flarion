use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

pub const REQUESTS_TOTAL: &str = "flarion_requests_total";
pub const ROUTE_RULE_MATCHES_TOTAL: &str = "flarion_route_rule_matches_total";
pub const FALLBACKS_TOTAL: &str = "flarion_fallbacks_total";
pub const ROUTE_EXHAUSTED_TOTAL: &str = "flarion_route_exhausted_total";

pub const FIRST_TOKEN_SECONDS: &str = "flarion_first_token_seconds";
pub const REQUEST_DURATION_SECONDS: &str = "flarion_request_duration_seconds";
pub const PROMPT_TOKENS: &str = "flarion_prompt_tokens";
pub const COMPLETION_TOKENS: &str = "flarion_completion_tokens";

#[allow(dead_code)]
pub const BUILD_INFO: &str = "flarion_build_info";

const SECONDS_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0];
const TOKEN_BUCKETS: &[f64] = &[16.0, 64.0, 256.0, 1024.0, 4096.0, 16384.0, 65536.0];

/// Build and install the Prometheus exporter. Returns the handle used by the
/// `/metrics` route. Should be called at most once per process.
pub fn install() -> Result<Arc<PrometheusHandle>, String> {
    let mut builder = PrometheusBuilder::new();
    for name in [FIRST_TOKEN_SECONDS, REQUEST_DURATION_SECONDS] {
        builder = builder
            .set_buckets_for_metric(
                metrics_exporter_prometheus::Matcher::Full(name.into()),
                SECONDS_BUCKETS,
            )
            .map_err(|e| format!("set_buckets_for_metric({name}): {e}"))?;
    }
    for name in [PROMPT_TOKENS, COMPLETION_TOKENS] {
        builder = builder
            .set_buckets_for_metric(
                metrics_exporter_prometheus::Matcher::Full(name.into()),
                TOKEN_BUCKETS,
            )
            .map_err(|e| format!("set_buckets_for_metric({name}): {e}"))?;
    }
    let handle = builder
        .install_recorder()
        .map_err(|e| format!("install_recorder: {e}"))?;

    metrics::describe_gauge!(
        "flarion_backend_poisoned",
        "1 if the local backend's worker thread has panicked, 0 otherwise"
    );
    metrics::describe_gauge!(
        "flarion_vram_budget_mb",
        "Configured VRAM budget for local model scheduling, in MB"
    );
    metrics::describe_gauge!(
        "flarion_vram_reserved_mb",
        "Current reserved VRAM for a specific model, in MB"
    );
    metrics::describe_counter!(
        "flarion_model_loads_total",
        "Model load attempts, labeled by outcome (success|over_budget|load_failed)"
    );
    metrics::describe_counter!(
        "flarion_model_unloads_total",
        "Model unload attempts, labeled by outcome (success|failed)"
    );
    metrics::describe_counter!(
        "flarion_model_evictions_total",
        "Model evictions triggered to free VRAM, labeled by reason (lru)"
    );

    metrics::gauge!(BUILD_INFO, "version" => env!("CARGO_PKG_VERSION").to_string()).set(1.0);

    Ok(Arc::new(handle))
}

/// Set the `flarion_backend_poisoned{model=...}` gauge.
pub fn set_backend_poisoned(model_id: &str, poisoned: bool) {
    let value: f64 = if poisoned { 1.0 } else { 0.0 };
    metrics::gauge!("flarion_backend_poisoned", "model" => model_id.to_string()).set(value);
}

/// Set the `flarion_vram_budget_mb` gauge. Call once at startup.
pub fn set_vram_budget(budget_mb: u64) {
    metrics::gauge!("flarion_vram_budget_mb").set(budget_mb as f64);
}

/// Set the `flarion_vram_reserved_mb{model=...}` gauge for a model.
pub fn set_vram_reserved(model_id: &str, mb: u64) {
    metrics::gauge!("flarion_vram_reserved_mb", "model" => model_id.to_string()).set(mb as f64);
}

/// GET /metrics handler — renders the current snapshot in Prometheus text format.
pub async fn metrics_handler(State(handle): State<Arc<PrometheusHandle>>) -> Response {
    (StatusCode::OK, handle.render()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    // install() can't run twice in one process (recorder conflict), so this
    // is a smoke-test on the exported names only.
    #[test]
    fn constants_have_flarion_prefix() {
        assert!(REQUESTS_TOTAL.starts_with("flarion_"));
        assert!(FIRST_TOKEN_SECONDS.starts_with("flarion_"));
        assert!(BUILD_INFO.starts_with("flarion_"));
        assert!(ROUTE_RULE_MATCHES_TOTAL.starts_with("flarion_"));
        assert!(FALLBACKS_TOTAL.starts_with("flarion_"));
        assert!(ROUTE_EXHAUSTED_TOTAL.starts_with("flarion_"));
        assert!(REQUEST_DURATION_SECONDS.starts_with("flarion_"));
        assert!(PROMPT_TOKENS.starts_with("flarion_"));
        assert!(COMPLETION_TOKENS.starts_with("flarion_"));
    }
}
