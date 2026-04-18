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
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

pub async fn get_models(State(state): State<Arc<AdminState>>) -> Json<Vec<Model>> {
    Json(model_snapshot(&state))
}

pub async fn post_load(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(backend) = state.registry.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("model not found: {id}")})),
        )
            .into_response();
    };
    match backend.load().await {
        Ok(()) => (StatusCode::OK, Json(json!({"loaded": id}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn post_unload(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(backend) = state.registry.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("model not found: {id}")})),
        )
            .into_response();
    };
    if state.tracker.in_flight(&id) > 0 {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "backend busy",
                "in_flight": state.tracker.in_flight(&id),
            })),
        )
            .into_response();
    }
    match backend.unload().await {
        Ok(()) => (StatusCode::OK, Json(json!({"unloaded": id}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn post_pin(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    pin_impl(&state, &id, true).await
}

pub async fn post_unpin(
    State(state): State<Arc<AdminState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    pin_impl(&state, &id, false).await
}

async fn pin_impl(state: &Arc<AdminState>, id: &str, pinned: bool) -> axum::response::Response {
    let Some(backend) = state.registry.get(id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("model not found: {id}")})),
        )
            .into_response();
    };
    match backend.pin(pinned).await {
        Ok(()) => (StatusCode::OK, Json(json!({"id": id, "pinned": pinned}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
