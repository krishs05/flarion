use axum::Json;
use axum::extract::State;
use std::sync::Arc;

use crate::api::types::{ModelObject, ModelsResponse};
use crate::engine::registry::BackendRegistry;

pub async fn list_models(State(registry): State<Arc<BackendRegistry>>) -> Json<ModelsResponse> {
    let now = chrono::Utc::now().timestamp();
    let data: Vec<ModelObject> = registry
        .model_infos()
        .into_iter()
        .map(|info| ModelObject {
            id: info.id,
            object: "model".to_string(),
            created: now,
            owned_by: info.provider,
        })
        .collect();

    Json(ModelsResponse {
        object: "list".to_string(),
        data,
    })
}
