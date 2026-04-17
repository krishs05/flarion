//! Phase 2e: true-streaming integration tests. Uses MockBackend (no GPU).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use flarion::engine::backend::InferenceBackend;
use flarion::engine::registry::BackendRegistry;
use flarion::engine::testing::MockBackend;
use futures_util::StreamExt;
use tower::ServiceExt;

fn build_app_with_backend(backend: Arc<dyn InferenceBackend>) -> axum::Router {
    let mut registry = BackendRegistry::new();
    registry.insert("mock".into(), backend);
    let registry = Arc::new(registry);

    let config = flarion::config::FlarionConfig::default();
    flarion::server::create_router(registry, &config.server, &config.metrics, None)
}

fn chat_request_body(stream: bool) -> String {
    serde_json::json!({
        "model": "mock",
        "messages": [{"role": "user", "content": "hi"}],
        "stream": stream,
    })
    .to_string()
}

#[tokio::test]
async fn stream_delivers_chunks_progressively() {
    let backend = Arc::new(MockBackend::streaming_paced(
        "mock",
        vec!["a".into(), "b".into(), "c".into(), "d".into()],
        Duration::from_millis(50),
    ));
    let app = build_app_with_backend(backend);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(chat_request_body(true)))
        .unwrap();

    let start = Instant::now();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let mut body_stream = resp.into_body().into_data_stream();
    let mut first_chunk_at: Option<Duration> = None;
    let mut chunk_count = 0;
    while let Some(chunk) = body_stream.next().await {
        let _bytes = chunk.unwrap();
        chunk_count += 1;
        if first_chunk_at.is_none() {
            first_chunk_at = Some(start.elapsed());
        }
    }

    assert!(
        chunk_count > 1,
        "expected multiple SSE chunks, got {chunk_count}"
    );
    let first_at = first_chunk_at.expect("no chunks received");
    assert!(
        first_at < Duration::from_millis(500),
        "first chunk arrived after {first_at:?}, not progressively"
    );
}

#[tokio::test]
async fn stream_client_disconnect_cancels_backend() {
    let backend = Arc::new(MockBackend::streaming_paced(
        "mock",
        vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
        Duration::from_millis(200),
    ));
    let backend_ref: Arc<MockBackend> = backend.clone();
    let app = build_app_with_backend(backend);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(chat_request_body(true)))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read one chunk then drop body stream — simulates client disconnect.
    let mut body_stream = resp.into_body().into_data_stream();
    let _first = body_stream.next().await.unwrap().unwrap();
    drop(body_stream);

    // Wait long enough for the next send attempt to fail.
    tokio::time::sleep(Duration::from_millis(400)).await;

    assert!(
        backend_ref.cancel_observed(),
        "backend should have observed client disconnect"
    );
}

#[tokio::test]
async fn stream_completes_with_finish_reason() {
    let backend = Arc::new(MockBackend::streaming_chunks(
        "mock",
        vec!["all".into(), " done".into()],
    ));
    let app = build_app_with_backend(backend);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(chat_request_body(true)))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let mut body_stream = resp.into_body().into_data_stream();
    let mut body_text = String::new();
    while let Some(chunk) = body_stream.next().await {
        let bytes = chunk.unwrap();
        body_text.push_str(std::str::from_utf8(&bytes).unwrap());
    }

    assert!(
        body_text.contains("\"finish_reason\":\"stop\""),
        "got:\n{body_text}"
    );
    assert!(body_text.contains("[DONE]") || body_text.contains("finish_reason"));
}
