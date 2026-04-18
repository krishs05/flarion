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
