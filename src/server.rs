use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::engine::backend::InferenceBackend;

pub fn create_router(backend: Arc<dyn InferenceBackend>) -> Router {
    Router::new()
        .route("/health", get(crate::api::health::health_check))
        .route("/v1/models", get(crate::api::models::list_models))
        .route(
            "/v1/chat/completions",
            post(crate::api::chat::chat_completions),
        )
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(CorsLayer::permissive())
        .with_state(backend)
}
