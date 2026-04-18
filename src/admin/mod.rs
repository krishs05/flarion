pub mod gpus;
pub mod models;
pub mod routes;
pub mod state;
pub mod status;
pub mod tracker;
pub mod types;
pub mod version;

use axum::Router;
use axum::routing::get;
use std::sync::Arc;

use crate::admin::state::AdminState;

pub fn admin_router(state: Arc<AdminState>) -> Router {
    Router::new()
        .route("/v1/admin/version", get(version::get_version))
        .route("/v1/admin/status", get(status::get_status))
        .with_state(state)
}
