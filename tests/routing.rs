//! End-to-end routing tests. Exercises RoutedBackend through the Axum handler
//! with mocked leaf backends. No real model loading; no real HTTP upstreams.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use flarion::api::types::{ChatCompletionRequest, ChatMessage};
use flarion::config::{Matchers, MetricsConfig, ServerConfig};
use flarion::engine::backend::InferenceBackend;
use flarion::engine::registry::BackendRegistry;
use flarion::engine::testing::MockBackend;
use flarion::error::EngineError;
use flarion::routing::matchers::CompiledMatchers;
use flarion::routing::routed_backend::RoutedBackend;
use flarion::routing::rules::{CompiledRoute, CompiledRule};
use flarion::server::create_router;

fn rule_default(
    name: &str,
    targets: Vec<Arc<dyn InferenceBackend>>,
    target_ids: Vec<&str>,
    timeout: Duration,
) -> CompiledRule {
    CompiledRule {
        name: name.into(),
        matchers: CompiledMatchers::compile(&Matchers::default()).unwrap(),
        targets,
        target_ids: target_ids.iter().map(|s| s.to_string()).collect(),
        first_token_timeout: timeout,
    }
}

fn request_body(model: &str, stream: bool) -> String {
    serde_json::to_string(&serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "hello"}],
        "stream": stream,
        "temperature": 0.7,
        "top_p": 0.9,
        "max_tokens": 16,
        "stop": [],
        "seed": null,
    }))
    .unwrap()
}

// Silence unused-import warnings: we import the request types in case future
// tests want typed builders, but the body helper uses serde_json::json! for
// forward-compat with optional fields.
#[allow(dead_code)]
fn _assert_types(_r: ChatCompletionRequest, _m: ChatMessage) {}

#[tokio::test]
async fn non_streaming_fallback_returns_second_backend_response() {
    let a = Arc::new(MockBackend::failing(
        "a",
        EngineError::Network("down".into()),
    )) as Arc<dyn InferenceBackend>;
    let b = Arc::new(MockBackend::succeeding("b", "from-b")) as Arc<dyn InferenceBackend>;

    let route = CompiledRoute {
        id: "chat".into(),
        rules: vec![rule_default(
            "default",
            vec![a.clone(), b.clone()],
            vec!["a", "b"],
            Duration::from_secs(5),
        )],
    };
    let routed = Arc::new(RoutedBackend::new(route, 4096)) as Arc<dyn InferenceBackend>;

    let mut registry = BackendRegistry::new();
    registry.insert("a".into(), a);
    registry.insert("b".into(), b);
    registry.insert("chat".into(), routed);
    let registry = Arc::new(registry);

    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(request_body("chat", false)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let fallback_header = response
        .headers()
        .get("x-flarion-fallback-count")
        .expect("fallback count header missing");
    assert_eq!(fallback_header.to_str().unwrap(), "1");

    let route_header = response.headers().get("x-flarion-route").unwrap();
    assert_eq!(route_header.to_str().unwrap(), "chat");

    let backend_header = response.headers().get("x-flarion-backend").unwrap();
    assert_eq!(backend_header.to_str().unwrap(), "b");

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["choices"][0]["message"]["content"].as_str().unwrap(),
        "from-b"
    );
}

#[tokio::test]
async fn direct_backend_access_still_works() {
    let a = Arc::new(MockBackend::succeeding("a", "direct-a")) as Arc<dyn InferenceBackend>;
    let b = Arc::new(MockBackend::succeeding("b", "direct-b")) as Arc<dyn InferenceBackend>;
    let route = CompiledRoute {
        id: "chat".into(),
        rules: vec![rule_default(
            "default",
            vec![a.clone()],
            vec!["a"],
            Duration::from_secs(5),
        )],
    };
    let routed = Arc::new(RoutedBackend::new(route, 4096)) as Arc<dyn InferenceBackend>;

    let mut registry = BackendRegistry::new();
    registry.insert("a".into(), a);
    registry.insert("b".into(), b);
    registry.insert("chat".into(), routed);
    let registry = Arc::new(registry);

    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(request_body("b", false)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let route_header = response.headers().get("x-flarion-route").unwrap();
    assert_eq!(route_header.to_str().unwrap(), "direct");
    let backend_header = response.headers().get("x-flarion-backend").unwrap();
    assert_eq!(backend_header.to_str().unwrap(), "b");
}

#[tokio::test]
async fn all_backends_failed_returns_502() {
    let a = Arc::new(MockBackend::failing("a", EngineError::Timeout)) as Arc<dyn InferenceBackend>;
    let b = Arc::new(MockBackend::failing(
        "b",
        EngineError::Network("gone".into()),
    )) as Arc<dyn InferenceBackend>;

    let route = CompiledRoute {
        id: "chat".into(),
        rules: vec![rule_default(
            "default",
            vec![a.clone(), b.clone()],
            vec!["a", "b"],
            Duration::from_secs(5),
        )],
    };
    let routed = Arc::new(RoutedBackend::new(route, 4096)) as Arc<dyn InferenceBackend>;

    let mut registry = BackendRegistry::new();
    registry.insert("a".into(), a);
    registry.insert("b".into(), b);
    registry.insert("chat".into(), routed);
    let registry = Arc::new(registry);

    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(request_body("chat", false)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "server_error");
    assert_eq!(json["error"]["code"], "upstream_error");
}

#[tokio::test]
async fn streaming_first_token_timeout_falls_back() {
    let slow = Arc::new(MockBackend::timing_out("slow", Duration::from_secs(60)))
        as Arc<dyn InferenceBackend>;
    let fast = Arc::new(MockBackend::streaming_chunks("fast", vec!["hi".into()]))
        as Arc<dyn InferenceBackend>;

    let route = CompiledRoute {
        id: "chat".into(),
        rules: vec![rule_default(
            "default",
            vec![slow.clone(), fast.clone()],
            vec!["slow", "fast"],
            Duration::from_millis(100),
        )],
    };
    let routed = Arc::new(RoutedBackend::new(route, 4096)) as Arc<dyn InferenceBackend>;

    let mut registry = BackendRegistry::new();
    registry.insert("slow".into(), slow);
    registry.insert("fast".into(), fast);
    registry.insert("chat".into(), routed);
    let registry = Arc::new(registry);

    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(request_body("chat", true)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        body_str.contains("\"hi\""),
        "expected fast-backend content in body, got: {body_str}"
    );
    assert!(body_str.contains("[DONE]"));
}

#[tokio::test]
async fn mid_stream_failure_does_not_invoke_fallback() {
    let primary = Arc::new(MockBackend::streaming_then_error(
        "primary",
        vec!["one".into(), "two".into()],
        EngineError::Network("broke".into()),
    )) as Arc<dyn InferenceBackend>;
    let fallback = Arc::new(MockBackend::streaming_chunks(
        "fallback",
        vec!["SHOULD_NOT_APPEAR".into()],
    )) as Arc<dyn InferenceBackend>;

    let route = CompiledRoute {
        id: "chat".into(),
        rules: vec![rule_default(
            "default",
            vec![primary.clone(), fallback.clone()],
            vec!["primary", "fallback"],
            Duration::from_secs(5),
        )],
    };
    let routed = Arc::new(RoutedBackend::new(route, 4096)) as Arc<dyn InferenceBackend>;

    let mut registry = BackendRegistry::new();
    registry.insert("primary".into(), primary);
    registry.insert("fallback".into(), fallback);
    registry.insert("chat".into(), routed);
    let registry = Arc::new(registry);

    let app = create_router(
        registry,
        &ServerConfig::default(),
        &MetricsConfig::default(),
        None,
    );

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(request_body("chat", true)))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        !body_str.contains("SHOULD_NOT_APPEAR"),
        "fallback must not be invoked after first chunk sent"
    );
    assert!(
        body_str.contains("\"one\"") && body_str.contains("\"two\""),
        "primary's two chunks should have reached the client"
    );
}
