use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use metrics_exporter_prometheus::PrometheusHandle;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::admin::admin_router;
use crate::auth::{AuthState, auth_middleware};
use crate::config::{MetricsConfig, ServerConfig};
use crate::engine::registry::BackendRegistry;

/// Maximum accepted JSON request body, in bytes. Chat completions shouldn't
/// legitimately exceed this; large system prompts + context can reach a few
/// hundred KB but 1 MiB is a comfortable ceiling that blocks obvious abuse.
const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

/// Build a CORS layer from server config. Three branches, in order:
/// 1. Non-empty `cors_origins` → that allow-list verbatim, GET/POST/OPTIONS.
/// 2. Empty list + loopback bind → permissive (dev UX).
/// 3. Empty list + public bind → deny all cross-origin.
fn build_cors_layer(server: &ServerConfig) -> CorsLayer {
    if !server.cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = server
            .cors_origins
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();
        return CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
    }
    if server.binds_loopback() {
        return CorsLayer::permissive();
    }
    CorsLayer::new()
}

/// Build the API sub-router with state already resolved to `Router<()>`.
///
/// Mounts `/health`, `/v1/models`, `/v1/chat/completions`, and — when
/// `metrics_cfg` says so — the metrics endpoint on the main listener.
/// Returning `Router<()>` lets callers merge additional routers (e.g. admin)
/// before adding shared middleware layers.
fn api_sub_router(
    registry: Arc<BackendRegistry>,
    metrics_cfg: &MetricsConfig,
    metrics_handle: Option<Arc<PrometheusHandle>>,
) -> Router {
    let mut app = Router::new()
        .route("/health", get(crate::api::health::health_check))
        .route("/v1/models", get(crate::api::models::list_models))
        .route(
            "/v1/chat/completions",
            post(crate::api::chat::chat_completions),
        );

    // Only mount /metrics here if no dedicated bind is configured; otherwise
    // main.rs owns a separate listener and gates it accordingly.
    if metrics_cfg.enabled
        && metrics_cfg.bind.is_none()
        && let Some(handle) = metrics_handle
    {
        app = app.route(
            metrics_cfg.path.as_str(),
            get(crate::metrics::metrics_handler).with_state(handle),
        );
    }

    app.with_state(registry)
}

pub fn create_router(
    registry: Arc<BackendRegistry>,
    server_cfg: &ServerConfig,
    metrics_cfg: &MetricsConfig,
    metrics_handle: Option<Arc<PrometheusHandle>>,
) -> Router {
    let auth_state = AuthState {
        api_keys: Arc::new(server_cfg.api_keys.clone()),
    };

    api_sub_router(registry, metrics_cfg, metrics_handle)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(from_fn_with_state(auth_state, auth_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(build_cors_layer(server_cfg))
}

pub fn create_router_with_admin(
    registry: Arc<BackendRegistry>,
    admin_state: Arc<crate::admin::state::AdminState>,
    server_cfg: &ServerConfig,
    metrics_cfg: &MetricsConfig,
    metrics_handle: Option<Arc<PrometheusHandle>>,
) -> Router {
    let auth_state = AuthState {
        api_keys: Arc::new(server_cfg.api_keys.clone()),
    };

    // Merge the admin router into the already-state-resolved API sub-router,
    // producing a single `Router<()>` before shared middleware is applied.
    let app = api_sub_router(registry, metrics_cfg, metrics_handle)
        .merge(admin_router(admin_state));

    app.layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(from_fn_with_state(auth_state, auth_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(300),
        ))
        .layer(build_cors_layer(server_cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    // CorsLayer's internals aren't introspectable, so these only exercise the
    // three selection branches. End-to-end CORS is covered in tests/security.rs.

    #[test]
    fn build_cors_explicit_list_branch() {
        let s = ServerConfig {
            cors_origins: vec!["https://app.example.com".into()],
            ..ServerConfig::default()
        };
        let _ = build_cors_layer(&s);
    }

    #[test]
    fn build_cors_loopback_permissive_branch() {
        let s = ServerConfig {
            host: "127.0.0.1".into(),
            cors_origins: Vec::new(),
            ..ServerConfig::default()
        };
        let _ = build_cors_layer(&s);
    }

    #[test]
    fn build_cors_public_deny_branch() {
        let s = ServerConfig {
            host: "0.0.0.0".into(),
            cors_origins: Vec::new(),
            ..ServerConfig::default()
        };
        let _ = build_cors_layer(&s);
    }
}
