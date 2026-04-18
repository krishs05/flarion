use std::sync::Arc;

use crate::admin::state::AdminState;
use crate::admin::types::Model;

pub fn model_snapshot(state: &Arc<AdminState>) -> Vec<Model> {
    state.registry.model_infos().into_iter().map(|info| {
        let backend = info.provider.to_string();
        let state_str = if info.loaded {
            "loaded"
        } else if backend == "openai" || backend == "groq" || backend == "anthropic" {
            "remote"
        } else {
            "unloaded"
        };
        Model {
            id: info.id.clone(),
            backend,
            state: state_str.into(),
            pinned: false,
            lazy: false,
            vram_mb: None,
            gpus: Vec::new(),
            in_flight: state.tracker.in_flight(&info.id),
            last_used_s: None,
        }
    }).collect()
}

use axum::Json;
use axum::extract::State;

pub async fn get_models(State(state): State<Arc<AdminState>>) -> Json<Vec<Model>> {
    Json(model_snapshot(&state))
}
