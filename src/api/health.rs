use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::engine::registry::BackendRegistry;

/// GET /health
///
/// Returns a minimal liveness signal only. `/health` is unauthenticated by
/// policy (see auth.rs) so we deliberately do NOT expose the running version,
/// loaded model ids, or per-model load state here — those reveal attack
/// surface to unauthenticated probers. Authed operators can read the same
/// information from `/v1/models` and logs.
pub async fn health_check(State(registry): State<Arc<BackendRegistry>>) -> Json<Value> {
    let infos = registry.model_infos();
    let all_healthy = !infos.is_empty() && infos.iter().all(|i| i.loaded);
    Json(json!({
        "status": if all_healthy { "ok" } else { "degraded" }
    }))
}
