use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::admin::state::AdminState;
use crate::admin::types::{Route, RouteRule};

pub fn route_snapshot(state: &Arc<AdminState>) -> Vec<Route> {
    state
        .routes
        .iter()
        .map(|cfg| Route {
            id: cfg.id.clone(),
            // Counters are placeholder in Phase 1; real wiring into the routing
            // layer is a follow-up — contract is forward-compatible.
            fallback_count: 0,
            rules: cfg
                .rules
                .iter()
                .map(|r| RouteRule {
                    name: r.name.clone(),
                    hit_count: 0,
                    targets: r.targets.clone(),
                })
                .collect(),
        })
        .collect()
}

pub async fn get_routes(State(state): State<Arc<AdminState>>) -> Json<Vec<Route>> {
    Json(route_snapshot(&state))
}
