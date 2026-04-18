use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;
use std::sync::Arc;

use flarion::admin::state::AdminState;
use flarion::config::{MetricsConfig, ServerConfig};
use flarion::engine::registry::BackendRegistry;
use flarion::server::create_router_with_admin;

fn make_admin_state(registry: Arc<BackendRegistry>) -> Arc<AdminState> {
    Arc::new(AdminState::new(registry, "127.0.0.1:0".to_string(), 1000))
}

#[tokio::test]
async fn admin_ping_requires_auth_when_keys_set() {
    let server_cfg = ServerConfig {
        api_keys: vec!["k".into()],
        ..ServerConfig::default()
    };
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &server_cfg,
        &MetricsConfig::default(),
        None,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_ping_passes_with_key() {
    let server_cfg = ServerConfig {
        api_keys: vec!["k".into()],
        ..ServerConfig::default()
    };
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &server_cfg,
        &MetricsConfig::default(),
        None,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/ping")
                .header(header::AUTHORIZATION, "Bearer k")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.status(), StatusCode::OK);
}
