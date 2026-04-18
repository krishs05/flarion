use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use tower::ServiceExt;
use std::sync::Arc;

use flarion::admin::state::AdminState;
use flarion::config::{MetricsConfig, ServerConfig};
use flarion::engine::registry::BackendRegistry;
use flarion::engine::testing::MockBackend;
use flarion::server::create_router_with_admin;

fn make_admin_state(registry: Arc<BackendRegistry>) -> Arc<AdminState> {
    Arc::new(AdminState::new(
        registry,
        Vec::new(),
        Arc::new(flarion::config::FlarionConfig::default()),
        "127.0.0.1:0".to_string(),
        1000,
    ))
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

#[tokio::test]
async fn admin_requests_tail_returns_most_recent() {
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    admin.tracker.record(flarion::admin::types::RequestEvent::Started {
        id: "a".into(),
        ts: "2026-04-18T00:00:00Z".into(),
        route: None,
        backend: "m".into(),
    }).await;
    admin.tracker.record(flarion::admin::types::RequestEvent::Started {
        id: "b".into(),
        ts: "2026-04-18T00:00:01Z".into(),
        route: None,
        backend: "m".into(),
    }).await;

    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/requests?tail=1").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: Vec<flarion::admin::types::RequestEvent> = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.len(), 1);
    if let flarion::admin::types::RequestEvent::Started { id, .. } = &v[0] {
        assert_eq!(id, "b");
    } else { panic!("expected Started"); }
}

#[tokio::test]
async fn admin_requests_tail_default_caps_at_1000() {
    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    // No tail param — should default to 50 (empty buffer → empty array)
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/requests").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: Vec<flarion::admin::types::RequestEvent> = serde_json::from_slice(&body).unwrap();
    assert!(v.is_empty());
}

#[tokio::test]
async fn admin_requests_stream_delivers_events() {
    use http_body_util::BodyExt;

    let registry = Arc::new(BackendRegistry::new());
    let admin = make_admin_state(registry.clone());
    let admin_for_push = admin.clone();

    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    // Start the subscribe request. It returns immediately with the SSE
    // response; we read frames from the body.
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/requests/stream").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Push an event after the subscription is live.
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        admin_for_push.tracker.record(flarion::admin::types::RequestEvent::Started {
            id: "s".into(),
            ts: "2026-04-18T00:00:00Z".into(),
            route: None,
            backend: "m".into(),
        }).await;
    });

    // Read the first SSE frame.
    let mut body = resp.into_body();
    let frame = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        body.frame(),
    ).await.expect("timeout waiting for SSE frame").expect("no frame").expect("frame err");
    let bytes = frame.into_data().expect("data frame");
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("data:"), "expected 'data:' in SSE frame, got: {text}");
    assert!(text.contains("\"event\":\"started\""), "expected started event, got: {text}");
}

#[tokio::test]
async fn admin_routes_returns_configured_routes() {
    use flarion::config::{RouteConfig, RuleConfig, Matchers};

    let registry = Arc::new(BackendRegistry::new());
    let admin = Arc::new(AdminState::new(
        registry.clone(),
        vec![RouteConfig {
            id: "chat".into(),
            first_token_timeout_ms: None,
            rules: vec![RuleConfig {
                name: "fallback".into(),
                matchers: Matchers::default(),
                targets: vec!["small".into()],
                first_token_timeout_ms: None,
            }],
        }],
        Arc::new(flarion::config::FlarionConfig::default()),
        "127.0.0.1:0".to_string(),
        1000,
    ));

    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/routes").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: Vec<flarion::admin::types::Route> = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].id, "chat");
    assert_eq!(v[0].rules.len(), 1);
    assert_eq!(v[0].rules[0].name, "fallback");
    assert_eq!(v[0].rules[0].hit_count, 0);
    assert_eq!(v[0].fallback_count, 0);
}

#[tokio::test]
async fn admin_routes_empty_when_no_routes_configured() {
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
        Request::builder().uri("/v1/admin/routes").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: Vec<flarion::admin::types::Route> = serde_json::from_slice(&body).unwrap();
    assert!(v.is_empty());
}

#[tokio::test]
async fn admin_config_redacts_api_keys() {
    use flarion::config::{FlarionConfig, ServerConfig};

    let cfg = FlarionConfig {
        server: ServerConfig {
            api_keys: vec!["secret-key".into()],
            ..ServerConfig::default()
        },
        ..FlarionConfig::default()
    };
    let registry = Arc::new(BackendRegistry::new());
    let admin = Arc::new(AdminState::new(
        registry.clone(),
        Vec::new(),
        Arc::new(cfg.clone()),
        "127.0.0.1:0".to_string(),
        1000,
    ));
    // Pass ServerConfig::default() (no api_keys) to the router so no auth is
    // required for the test request; the config stored in AdminState still
    // contains the secret key that should be redacted.
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    let resp = app.oneshot(
        Request::builder().uri("/v1/admin/config").body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let keys = v.pointer("/server/api_keys").and_then(|k| k.as_array()).expect("api_keys array");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].as_str().unwrap(), "***");
}

fn make_registry_with(id: &str, reply: &str) -> Arc<BackendRegistry> {
    let mut reg = BackendRegistry::new();
    reg.insert(id.to_string(), Arc::new(MockBackend::succeeding(id, reply)));
    Arc::new(reg)
}

#[tokio::test]
async fn admin_load_unknown_model_returns_404() {
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
                .method(Method::POST)
                .uri("/v1/admin/models/nope/load")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_load_known_model_returns_200() {
    let registry = make_registry_with("m", "ok");
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
                .method(Method::POST)
                .uri("/v1/admin/models/m/load")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn admin_unload_busy_model_returns_409() {
    let registry = make_registry_with("m", "ok");
    let admin = make_admin_state(registry.clone());
    admin.tracker.in_flight_inc("m"); // simulate in-flight
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
                .method(Method::POST)
                .uri("/v1/admin/models/m/unload")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn admin_pin_idempotent() {
    let registry = make_registry_with("m", "ok");
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    for _ in 0..2 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/admin/models/m/pin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn admin_unpin_idempotent() {
    let registry = make_registry_with("m", "ok");
    let admin = make_admin_state(registry.clone());
    let app = create_router_with_admin(
        registry,
        admin,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );
    for _ in 0..2 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/admin/models/m/unpin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
