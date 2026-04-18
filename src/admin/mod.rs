pub mod state;
pub mod tracker;
pub mod types;

use axum::Router;
use axum::routing::get;
use std::sync::Arc;

use crate::admin::state::AdminState;

pub fn admin_router(state: Arc<AdminState>) -> Router {
    Router::new()
        .route("/v1/admin/ping", get(|| async { "admin" }))
        .with_state(state)
}
