use axum::Json;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct AuthState {
    pub api_keys: Arc<Vec<String>>,
}

/// Paths that are always open, even when `api_keys` is set. Keep this list
/// minimal — anything unauthenticated is a potential info-disclosure
/// surface. `/health` is kept open so load balancers and monitoring can
/// probe liveness without credentials.
const UNAUTHENTICATED_PATHS: &[&str] = &["/health"];

pub async fn auth_middleware(
    State(state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    if state.api_keys.is_empty() {
        return next.run(request).await;
    }

    let path = request.uri().path();
    if UNAUTHENTICATED_PATHS.contains(&path) {
        return next.run(request).await;
    }

    let header_value = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let presented_key = header_value
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);

    // Walk every configured key without short-circuiting: `ct_eq` is
    // per-comparison constant-time, and folding the Choice values with OR
    // makes the total iteration time independent of which (or whether any)
    // key matched — so timing can't reveal a key's position in the list.
    let valid = match presented_key {
        Some(presented) => {
            let mut matched = subtle::Choice::from(0u8);
            for k in state.api_keys.iter() {
                matched |= k.as_bytes().ct_eq(presented.as_bytes());
            }
            bool::from(matched)
        }
        None => false,
    };

    if valid {
        next.run(request).await
    } else {
        let body = json!({
            "error": {
                "message": "missing or invalid API key",
                "type": "authentication_error",
                "code": "invalid_api_key"
            }
        });
        (StatusCode::UNAUTHORIZED, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use tower::ServiceExt;

    fn router_with_auth(api_keys: Vec<String>) -> Router {
        let state = AuthState {
            api_keys: Arc::new(api_keys),
        };
        Router::new()
            .route("/health", get(|| async { "ok" }))
            .route("/v1/models", get(|| async { "models-ok" }))
            .route("/metrics", get(|| async { "metrics-ok" }))
            .layer(from_fn_with_state(state, auth_middleware))
    }

    #[tokio::test]
    async fn test_metrics_requires_auth_when_keys_set() {
        let app = router_with_auth(vec!["test-key".to_string()]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "/metrics must be gated behind the bearer token when api_keys is set"
        );
    }

    #[tokio::test]
    async fn test_metrics_open_when_no_keys() {
        let app = router_with_auth(vec![]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_no_keys_passes_through() {
        let app = router_with_auth(vec![]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_health_open_when_keys_set() {
        let app = router_with_auth(vec!["test-key".to_string()]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_v1_requires_bearer() {
        let app = router_with_auth(vec!["test-key".to_string()]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["type"], "authentication_error");
        assert_eq!(json["error"]["code"], "invalid_api_key");
    }

    #[tokio::test]
    async fn test_v1_accepts_valid_bearer() {
        let app = router_with_auth(vec!["test-key".to_string()]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/models")
                    .header(header::AUTHORIZATION, "Bearer test-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_v1_rejects_wrong_key() {
        let app = router_with_auth(vec!["right-key".to_string()]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/models")
                    .header(header::AUTHORIZATION, "Bearer wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_v1_accepts_one_of_multiple_keys() {
        let app = router_with_auth(vec!["key-a".to_string(), "key-b".to_string()]);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/v1/models")
                    .header(header::AUTHORIZATION, "Bearer key-b")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
