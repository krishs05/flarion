use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::admin::state::AdminState;
use crate::admin::types::{ServerInfo, Status};

pub async fn get_status(State(state): State<Arc<AdminState>>) -> Json<Status> {
    let mut features = Vec::new();
    if cfg!(feature = "cuda") {
        features.push("cuda".into());
    }
    if cfg!(feature = "hf_cuda") {
        features.push("hf_cuda".into());
    }

    let gpus = crate::admin::gpus::gpu_snapshot(&state);
    let models = crate::admin::models::model_snapshot(&state);
    let recent = state.tracker.recent_rollup().await;

    Json(Status {
        server: ServerInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            git_sha: option_env!("FLARION_GIT_SHA").map(String::from),
            uptime_s: state.uptime_s(),
            bind: state.bind.clone(),
            features,
        },
        in_flight_total: state.tracker.in_flight_total(),
        gpus,
        models,
        recent,
    })
}
