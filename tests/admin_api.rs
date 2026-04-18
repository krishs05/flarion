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
async fn admin_version_requires_auth_when_keys_set() {
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
                .uri("/v1/admin/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admin_version_passes_with_key() {
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
                .uri("/v1/admin/version")
                .header(header::AUTHORIZATION, "Bearer k")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_version_returns_build_info() {
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/admin/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: flarion::admin::types::BuildInfo = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.version, env!("CARGO_PKG_VERSION"));
    // features: should be empty (default build, no cuda/hf_cuda) or contain
    // exactly the features the test binary was compiled with.
    for f in &v.features {
        assert!(f == "cuda" || f == "hf_cuda", "unexpected feature: {f}");
    }
}

#[tokio::test]
async fn admin_status_returns_shape_with_empty_registry() {
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/status").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let s: flarion::admin::types::Status = serde_json::from_slice(&body).unwrap();
    assert_eq!(s.server.version, env!("CARGO_PKG_VERSION"));
    assert!(s.models.is_empty());
    assert_eq!(s.in_flight_total, 0);
    assert_eq!(s.recent.requests_last_60s, 0);
    assert_eq!(s.recent.errors_last_60s, 0);
}

#[tokio::test]
async fn admin_gpus_returns_array() {
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/gpus").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: Vec<flarion::admin::types::Gpu> = serde_json::from_slice(&body).unwrap();
    // On CPU-only test runs this is empty. On a CUDA build with a visible device it's non-empty.
    for g in &v { assert!(g.budget_mb > 0); }
}

#[tokio::test]
async fn admin_models_returns_empty_array_for_empty_registry() {
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/models").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: Vec<flarion::admin::types::Model> = serde_json::from_slice(&body).unwrap();
    assert!(v.is_empty());
}
