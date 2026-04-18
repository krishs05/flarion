use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::admin::state::AdminState;
use crate::admin::types::BuildInfo;

pub async fn get_version(State(_state): State<Arc<AdminState>>) -> Json<BuildInfo> {
    let mut features = Vec::new();
    if cfg!(feature = "cuda") {
        features.push("cuda".into());
    }
    if cfg!(feature = "hf_cuda") {
        features.push("hf_cuda".into());
    }
    Json(BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: option_env!("FLARION_GIT_SHA").map(String::from),
        features,
    })
}
