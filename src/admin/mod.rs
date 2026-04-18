pub mod config;
pub mod gpus;
pub mod models;
pub mod requests;
pub mod routes;
pub mod state;
pub mod status;
pub mod tracker;
pub mod types;
pub mod version;

use axum::Router;
use axum::routing::{get, post};
use std::sync::Arc;

use crate::admin::state::AdminState;

pub fn admin_router(state: Arc<AdminState>) -> Router {
    Router::new()
        .route("/v1/admin/version", get(version::get_version))
        .route("/v1/admin/status", get(status::get_status))
        .route("/v1/admin/gpus", get(gpus::get_gpus))
        .route("/v1/admin/models", get(models::get_models))
        .route("/v1/admin/models/{id}/load", post(models::post_load))
        .route("/v1/admin/models/{id}/unload", post(models::post_unload))
        .route("/v1/admin/models/{id}/pin", post(models::post_pin))
        .route("/v1/admin/models/{id}/unpin", post(models::post_unpin))
        .route("/v1/admin/requests", get(requests::get_requests))
        .route("/v1/admin/requests/stream", get(requests::stream_requests))
        .route("/v1/admin/routes", get(routes::get_routes))
        .route("/v1/admin/config", get(config::get_config))
        .with_state(state)
}
