use axum::Json;
use axum::extract::State;
use std::sync::Arc;

use crate::api::types::HealthResponse;
use crate::engine::backend::InferenceBackend;

pub async fn health_check(
    State(backend): State<Arc<dyn InferenceBackend>>,
) -> Json<HealthResponse> {
    let info = backend.model_info();
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        model_loaded: info.loaded,
        model_id: info.id,
    })
}
