use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use flarion::cli::{client::FlarionClient, endpoint::Endpoint, error::ClientError};

fn mk_endpoint(url: String) -> Endpoint {
    Endpoint {
        name: "test".into(),
        url,
        api_key: None,
    }
}

#[tokio::test]
async fn version_parses_build_info() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/admin/version"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "version": "0.9.0",
            "git_sha": null,
            "features": ["cuda"]
        })))
        .mount(&server)
        .await;

    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let v = c.version().await.unwrap();
    assert_eq!(v.version, "0.9.0");
    assert_eq!(v.git_sha, None);
    assert_eq!(v.features, vec!["cuda"]);
}

#[tokio::test]
async fn version_surfaces_401_as_unauthorized() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/admin/version"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let err = c.version().await.unwrap_err();
    assert!(matches!(err, ClientError::Unauthorized), "got: {err:?}");
}

#[tokio::test]
async fn version_surfaces_unreachable_for_bad_host() {
    let c = FlarionClient::new(Endpoint {
        name: "bad".into(),
        url: "http://127.0.0.1:1".into(),   // nothing listens on port 1
        api_key: None,
    }).unwrap();
    let err = c.version().await.unwrap_err();
    assert!(
        matches!(err, ClientError::Unreachable { .. } | ClientError::Timeout),
        "got: {err:?}"
    );
}

#[tokio::test]
async fn version_includes_bearer_auth_header_when_key_set() {
    use wiremock::matchers::header;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/admin/version"))
        .and(header("authorization", "Bearer my-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "version": "0.9.0", "git_sha": null, "features": []
        })))
        .mount(&server)
        .await;

    let c = FlarionClient::new(Endpoint {
        name: "t".into(),
        url: server.uri(),
        api_key: Some("my-key".into()),
    }).unwrap();
    c.version().await.unwrap();
    // If the header didn't match, wiremock would've returned 404 and the call would fail.
}

#[tokio::test]
async fn status_parses_with_minimal_shape() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "server": { "version": "0.9.0", "git_sha": null, "uptime_s": 10, "bind": "127.0.0.1:8080", "features": [] },
        "gpus": [],
        "models": [],
        "in_flight_total": 0,
        "recent": { "requests_last_60s": 0, "errors_last_60s": 0, "ttft_p50_ms": null, "ttft_p95_ms": null }
    });
    Mock::given(method("GET")).and(path("/v1/admin/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let s = c.status().await.unwrap();
    assert_eq!(s.server.version, "0.9.0");
    assert_eq!(s.in_flight_total, 0);
    assert!(s.gpus.is_empty());
}

#[tokio::test]
async fn gpus_parses_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/admin/gpus"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 0, "name": "Tesla V100", "budget_mb": 32000, "reserved_mb": 8000, "free_mb": 24000, "models": ["m"] }
        ])))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let g = c.gpus().await.unwrap();
    assert_eq!(g.len(), 1);
    assert_eq!(g[0].name, "Tesla V100");
}

#[tokio::test]
async fn models_parses_empty_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/admin/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let m = c.models().await.unwrap();
    assert!(m.is_empty());
}

#[tokio::test]
async fn routes_parses_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/admin/routes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": "chat", "fallback_count": 0, "rules": [] }
        ])))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let r = c.routes().await.unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].id, "chat");
}

#[tokio::test]
async fn requests_includes_tail_query() {
    use wiremock::matchers::query_param;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/admin/requests"))
        .and(query_param("tail", "25"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let r = c.requests(25).await.unwrap();
    assert!(r.is_empty());
}

#[tokio::test]
async fn effective_config_returns_json_value() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/admin/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "server": { "api_keys": ["***"] }
        })))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let v = c.effective_config().await.unwrap();
    assert_eq!(v.pointer("/server/api_keys/0").and_then(|v| v.as_str()), Some("***"));
}

#[tokio::test]
async fn health_returns_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})))
        .mount(&server).await;
    let c = FlarionClient::new(mk_endpoint(server.uri())).unwrap();
    let v = c.health().await.unwrap();
    assert_eq!(v.get("status").and_then(|v| v.as_str()), Some("ok"));
}
