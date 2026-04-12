use axum::extract::State;
use axum::Json;
use std::sync::Arc;

use crate::api::types::{ModelObject, ModelsResponse};
use crate::engine::backend::InferenceBackend;

pub async fn list_models(
    State(backend): State<Arc<dyn InferenceBackend>>,
) -> Json<ModelsResponse> {
    let info = backend.model_info();
    Json(ModelsResponse {
        object: "list".to_string(),
        data: vec![ModelObject {
            id: info.id,
            object: "model".to_string(),
            created: chrono::Utc::now().timestamp(),
            owned_by: "local".to_string(),
        }],
    })
}
