//! Security-focused verification tests. These lock in current defenses and
//! catch regressions — each test is tied to a specific threat in the pen-test
//! audit.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use flarion::config::{MetricsConfig, ServerConfig};
use flarion::engine::backend::InferenceBackend;
use flarion::engine::registry::BackendRegistry;
use flarion::engine::testing::MockBackend;
use flarion::server::create_router;

fn registry_with_one(id: &str) -> Arc<BackendRegistry> {
    let mut r = BackendRegistry::new();
    r.insert(
        id.to_string(),
        Arc::new(MockBackend::succeeding(id, "hi")) as Arc<dyn InferenceBackend>,
    );
    Arc::new(r)
}

// ── 413: Request body exceeds the 1 MiB DefaultBodyLimit ──────────────────

#[tokio::test]
async fn oversized_body_returns_413() {
    let registry = registry_with_one("m");
    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    // 2 MiB of pad. Must exceed MAX_REQUEST_BODY_BYTES = 1 MiB in server.rs.
    let big_content = "a".repeat(2 * 1024 * 1024);
    let body = serde_json::json!({
        "model": "m",
        "messages": [{"role": "user", "content": big_content}],
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "1 MiB body limit must return 413"
    );
}

// ── /health minimal response — no version or model leak ───────────────────

#[tokio::test]
async fn health_response_is_minimal() {
    let registry = registry_with_one("secret-internal-model-xyz");
    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Keys that MUST NOT appear: version, models, model_id, all_healthy.
    assert!(json.get("version").is_none(), "/health leaks version");
    assert!(
        json.get("models").is_none(),
        "/health leaks model inventory"
    );
    assert!(json.get("model_id").is_none(), "/health leaks model id");
    assert!(
        json.get("all_healthy").is_none(),
        "/health leaks per-model health"
    );

    // Must NOT contain the model name anywhere in the body.
    let body_str = json.to_string();
    assert!(
        !body_str.contains("secret-internal-model-xyz"),
        "model id leaked into /health body: {body_str}"
    );

    // Only `status` is allowed.
    assert!(json.get("status").is_some(), "/health missing status");
}

// ── 400 for empty messages array ──────────────────────────────────────────

#[tokio::test]
async fn empty_messages_returns_400() {
    let registry = registry_with_one("m");
    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let body = serde_json::json!({
        "model": "m",
        "messages": [],
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── 400 when messages.len() > MAX_MESSAGES ────────────────────────────────

#[tokio::test]
async fn too_many_messages_returns_400() {
    let registry = registry_with_one("m");
    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let messages: Vec<_> = (0..500)
        .map(|i| serde_json::json!({"role": "user", "content": format!("msg {i}")}))
        .collect();
    let body = serde_json::json!({
        "model": "m",
        "messages": messages,
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        body_str.contains("too many messages"),
        "expected message-count error, got: {body_str}"
    );
}

// ── max_tokens silently clamped (doesn't 400, request still succeeds) ─────

#[tokio::test]
async fn oversized_max_tokens_is_clamped_not_rejected() {
    // MockBackend::succeeding ignores max_tokens, so "succeed" = "clamp
    // happened before reaching the backend". If we had no clamp, the request
    // would still work against the mock, so this test guarantees only that
    // the CLAMP PATH compiles and the server doesn't 400 on large values.
    // A stronger test would require a mock that records the received value —
    // left as a phase-2d follow-up.
    let registry = registry_with_one("m");
    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let body = serde_json::json!({
        "model": "m",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 4_000_000_000u64,
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "server must clamp max_tokens silently, not reject with 400"
    );
}

// ── /metrics is gated behind auth when api_keys is set ────────────────────

#[tokio::test]
async fn metrics_requires_bearer_when_keys_set() {
    let registry = registry_with_one("m");
    let metrics_cfg = MetricsConfig {
        enabled: false, // we only need the auth layer to reject; /metrics route itself doesn't need to exist
        path: "/metrics".into(),
        bind: None,
    };
    let server_cfg = ServerConfig {
        api_keys: vec!["key-1".to_string()],
        ..ServerConfig::default()
    };
    let app = create_router(registry, &server_cfg, &metrics_cfg, None);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "/metrics must require a bearer when api_keys is set"
    );
}

// ── 405 Method Not Allowed on wrong verb for /v1/chat/completions ─────────

#[tokio::test]
async fn wrong_method_on_chat_completions_returns_405() {
    let registry = registry_with_one("m");
    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/v1/chat/completions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

// ── Auth response doesn't leak model inventory ────────────────────────────

#[tokio::test]
async fn auth_failure_does_not_leak_models() {
    let mut r = BackendRegistry::new();
    r.insert(
        "secret-internal-model".to_string(),
        Arc::new(MockBackend::succeeding("secret-internal-model", "hi"))
            as Arc<dyn InferenceBackend>,
    );
    let registry = Arc::new(r);

    let server_cfg = ServerConfig {
        api_keys: vec!["the-real-key".to_string()],
        ..ServerConfig::default()
    };
    let app = create_router(registry, &server_cfg, &MetricsConfig::default(), None);

    let body = serde_json::json!({
        "model": "secret-internal-model",
        "messages": [{"role": "user", "content": "hi"}],
    })
    .to_string();

    // No Authorization header — should 401 before model resolution.
    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        !body_str.contains("secret-internal-model"),
        "401 leaked a configured model id: {body_str}"
    );
}

// ── Phase 2d: CORS allow-list honored ─────────────────────────────────────

#[tokio::test]
async fn cors_allow_list_honors_configured_origin() {
    let registry = registry_with_one("m");
    let server_cfg = ServerConfig {
        host: "0.0.0.0".into(), // public bind — triggers strict CORS path
        cors_origins: vec!["https://app.example.com".into()],
        allow_unauthenticated: true, // skip auth for this test
        ..ServerConfig::default()
    };
    let app = create_router(registry, &server_cfg, &MetricsConfig::default(), None);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/chat/completions")
                .header("origin", "https://app.example.com")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        allow,
        Some("https://app.example.com"),
        "configured origin must be allowed"
    );
}

#[tokio::test]
async fn cors_allow_list_rejects_unlisted_origin() {
    let registry = registry_with_one("m");
    let server_cfg = ServerConfig {
        host: "0.0.0.0".into(),
        cors_origins: vec!["https://app.example.com".into()],
        allow_unauthenticated: true,
        ..ServerConfig::default()
    };
    let app = create_router(registry, &server_cfg, &MetricsConfig::default(), None);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/chat/completions")
                .header("origin", "https://attacker.example.com")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());
    assert_ne!(
        allow,
        Some("https://attacker.example.com"),
        "unlisted origin must NOT be echoed in access-control-allow-origin"
    );
}

// ── Phase 2d: CORS deny-all on public bind with empty list ────────────────

#[tokio::test]
async fn cors_public_bind_empty_list_rejects_all_cross_origin() {
    let registry = registry_with_one("m");
    let server_cfg = ServerConfig {
        host: "0.0.0.0".into(),
        cors_origins: Vec::new(),
        allow_unauthenticated: true,
        ..ServerConfig::default()
    };
    let app = create_router(registry, &server_cfg, &MetricsConfig::default(), None);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/v1/chat/completions")
                .header("origin", "https://anything.example.com")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let allow = resp.headers().get("access-control-allow-origin");
    assert!(
        allow.is_none(),
        "public bind with empty cors_origins must not set access-control-allow-origin"
    );
}

// ── Phase 2d: per-model max_tokens_cap clamp ──────────────────────────────

#[tokio::test]
async fn per_model_max_tokens_cap_is_honored() {
    // Model declares cap=1000. Request asks for 5000. Handler must clamp
    // before dispatching. MockBackend::succeeding ignores max_tokens, so we
    // verify no 400 AND no 500 — the request goes through cleanly.
    let mut r = BackendRegistry::new();
    let mock = MockBackend::succeeding("tiny", "ok").with_max_tokens_cap(1000);
    r.insert(
        "tiny".to_string(),
        Arc::new(mock) as Arc<dyn InferenceBackend>,
    );
    let registry = Arc::new(r);

    let server_cfg = ServerConfig::default();
    let app = create_router(registry, &server_cfg, &MetricsConfig::default(), None);

    let body = serde_json::json!({
        "model": "tiny",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 5000,
    })
    .to_string();

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "request above cap must be silently clamped (200), not rejected"
    );
}

// ── Phase 2d: /metrics not mounted on main listener when dedicated bind set ──

#[tokio::test]
async fn metrics_not_mounted_on_main_when_dedicated_bind_configured() {
    let registry = registry_with_one("m");
    let server_cfg = ServerConfig::default();
    let metrics_cfg = MetricsConfig {
        enabled: true,
        path: "/metrics".into(),
        bind: Some("127.0.0.1:0".into()), // any — not actually binding here
    };

    // We pass None for the handle because the main listener shouldn't mount
    // /metrics at all when a dedicated bind is configured (see server.rs
    // gating). The dedicated listener is tested at runtime by main.rs.
    let app = create_router(registry, &server_cfg, &metrics_cfg, None);

    let resp = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "/metrics must not be mounted on main listener when dedicated bind is configured"
    );
}
