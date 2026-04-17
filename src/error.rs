use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("model not found: {0}")]
    #[allow(dead_code)]
    ModelNotFound(String),
    #[error("failed to load model: {0}")]
    ModelLoadFailed(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("context length exceeded: requested {requested}, max {max}")]
    ContextLengthExceeded { requested: usize, max: usize },
    #[error("request timed out")]
    Timeout,
    #[error("network error: {0}")]
    Network(String),
    #[error("upstream server error ({status}): {body}")]
    UpstreamServerError { status: u16, body: String },
    #[error("rate limited{}", match .retry_after {
        Some(d) => format!(" (retry after {}s)", d.as_secs()),
        None => String::new(),
    })]
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    #[error("all backends failed for route '{route_id}' ({} attempts)", attempts.len())]
    #[allow(dead_code)]
    AllBackendsFailed {
        route_id: String,
        attempts: Vec<(String, Box<EngineError>)>,
    },
    #[error("backend poisoned by worker panic")]
    BackendPoisoned,

    #[error("backend draining (server shutting down)")]
    BackendDraining,

    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
}

#[allow(dead_code)]
pub fn is_retryable(err: &EngineError) -> bool {
    matches!(
        err,
        EngineError::Timeout
            | EngineError::Network(_)
            | EngineError::UpstreamServerError { .. }
            | EngineError::RateLimited { .. }
    )
    // BackendPoisoned / BackendDraining: not retryable (restart / shutdown).
    // ModelUnavailable: not retryable without eviction / capacity changes.
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("{}", format_model_not_found(.requested, .available))]
    ModelNotFound {
        requested: String,
        available: Vec<String>,
    },
    #[error("internal error: {0}")]
    Internal(String),
    #[error("{0}")]
    BadGateway(String),
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
}

fn format_model_not_found(requested: &str, available: &[String]) -> String {
    if available.is_empty() {
        format!("model '{requested}' not found; no models loaded")
    } else {
        format!(
            "model '{requested}' not found. available models: {}",
            available.join(", ")
        )
    }
}

impl From<EngineError> for ApiError {
    /// Maps engine errors to client-facing API errors. Internal details
    /// (upstream bodies, backend ids, per-attempt errors, engine strings) are
    /// logged at `error` level but never included in the client response.
    /// Clients get a stable, opaque message plus a stable `code` field they
    /// can branch on; operators get the full story in `tracing` output.
    fn from(err: EngineError) -> Self {
        match err {
            EngineError::ModelNotFound(id) => ApiError::ModelNotFound {
                requested: id,
                available: Vec::new(),
            },
            EngineError::ModelLoadFailed(msg) => {
                tracing::error!(error = %msg, "model load failed");
                ApiError::Internal("internal server error".into())
            }
            EngineError::InferenceFailed(msg) => {
                tracing::error!(error = %msg, "inference failed");
                ApiError::Internal("internal server error".into())
            }
            EngineError::ContextLengthExceeded { requested, max } => ApiError::BadRequest(format!(
                "context length exceeded: requested {requested}, max {max}"
            )),
            EngineError::Timeout => ApiError::BadGateway("upstream request timed out".into()),
            EngineError::Network(msg) => {
                tracing::error!(error = %msg, "upstream network error");
                ApiError::BadGateway("upstream unreachable".into())
            }
            EngineError::UpstreamServerError { status, body } => {
                tracing::error!(status = status, body = %body, "upstream server error");
                ApiError::BadGateway(format!("upstream error {status}"))
            }
            EngineError::RateLimited { .. } => ApiError::BadGateway("upstream rate limited".into()),
            EngineError::AllBackendsFailed { route_id, attempts } => {
                let summary = attempts
                    .iter()
                    .map(|(id, e)| format!("{id}: {e}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                tracing::error!(route = %route_id, attempts = %summary, "all backends in route failed");
                ApiError::BadGateway(format!("all backends failed for route '{route_id}'"))
            }
            EngineError::BackendPoisoned => {
                tracing::error!("backend poisoned, rejecting request");
                ApiError::ServiceUnavailable("model backend temporarily unavailable".into())
            }
            EngineError::BackendDraining => {
                tracing::warn!("backend draining, rejecting request");
                ApiError::ServiceUnavailable("server shutting down".into())
            }
            EngineError::ModelUnavailable(detail) => {
                tracing::warn!(%detail, "request rejected: model unavailable");
                ApiError::ServiceUnavailable("model temporarily unavailable".into())
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_type, code) = match &self {
            ApiError::BadRequest(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "bad_request",
            ),
            ApiError::ModelNotFound { .. } => (
                StatusCode::NOT_FOUND,
                "invalid_request_error",
                "model_not_found",
            ),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal_error",
            ),
            ApiError::BadGateway(_) => (StatusCode::BAD_GATEWAY, "server_error", "upstream_error"),
            ApiError::ServiceUnavailable(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "service_unavailable",
            ),
        };

        let body = json!({
            "error": {
                "message": self.to_string(),
                "type": error_type,
                "code": code,
            }
        });

        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    #[tokio::test]
    async fn test_bad_request_returns_400() {
        let err = ApiError::BadRequest("missing messages".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert_eq!(json["error"]["code"], "bad_request");
    }

    #[tokio::test]
    async fn test_model_not_found_returns_404() {
        let err = ApiError::ModelNotFound {
            requested: "nonexistent".into(),
            available: Vec::new(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_internal_error_returns_500() {
        let err = ApiError::Internal("something broke".into());
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_engine_error_converts_to_api_error() {
        let engine_err = EngineError::ModelNotFound("test-model".into());
        let api_err: ApiError = engine_err.into();
        assert!(matches!(api_err, ApiError::ModelNotFound { .. }));

        let engine_err = EngineError::ContextLengthExceeded {
            requested: 8000,
            max: 4096,
        };
        let api_err: ApiError = engine_err.into();
        assert!(matches!(api_err, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_model_not_found_lists_available_models() {
        let err = ApiError::ModelNotFound {
            requested: "gpt-4o".to_string(),
            available: vec!["qwen3-8b".to_string(), "codellama-13b".to_string()],
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let message = json["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("gpt-4o"),
            "message missing requested id: {message}"
        );
        assert!(
            message.contains("qwen3-8b"),
            "message missing available model: {message}"
        );
        assert!(
            message.contains("codellama-13b"),
            "message missing available model: {message}"
        );
    }

    #[tokio::test]
    async fn test_model_not_found_empty_available_list() {
        let err = ApiError::ModelNotFound {
            requested: "x".to_string(),
            available: Vec::new(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let message = json["error"]["message"].as_str().unwrap();
        assert!(message.contains("no models loaded"), "got: {message}");
    }

    #[test]
    fn test_is_retryable_true_for_retryable_variants() {
        assert!(is_retryable(&EngineError::Timeout));
        assert!(is_retryable(&EngineError::Network("conn reset".into())));
        assert!(is_retryable(&EngineError::UpstreamServerError {
            status: 503,
            body: "".into(),
        }));
        assert!(is_retryable(&EngineError::RateLimited {
            retry_after: None
        }));
    }

    #[test]
    fn test_is_retryable_false_for_terminal_variants() {
        assert!(!is_retryable(&EngineError::InferenceFailed("x".into())));
        assert!(!is_retryable(&EngineError::ModelNotFound("x".into())));
        assert!(!is_retryable(&EngineError::ModelLoadFailed("x".into())));
        assert!(!is_retryable(&EngineError::ContextLengthExceeded {
            requested: 10,
            max: 4,
        }));
    }

    #[tokio::test]
    async fn test_all_backends_failed_returns_502() {
        let err = EngineError::AllBackendsFailed {
            route_id: "chat".into(),
            attempts: vec![("backend-a".into(), Box::new(EngineError::Timeout))],
        };
        let api: ApiError = err.into();
        let response = api.into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["type"], "server_error");
        assert_eq!(json["error"]["code"], "upstream_error");
        let msg = json["error"]["message"].as_str().unwrap();
        assert!(msg.contains("chat"));
        // Per-attempt errors and backend ids must NOT be leaked to the client
        // — they are logged server-side. Only the route id should appear.
        assert!(!msg.contains("backend-a"));
    }

    #[tokio::test]
    async fn test_timeout_returns_502_bad_gateway() {
        let api: ApiError = EngineError::Timeout.into();
        let response = api.into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn test_rate_limited_returns_502() {
        let api: ApiError = EngineError::RateLimited {
            retry_after: Some(std::time::Duration::from_secs(5)),
        }
        .into();
        let response = api.into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_backend_poisoned_is_not_retryable() {
        assert!(!is_retryable(&EngineError::BackendPoisoned));
    }

    #[test]
    fn test_backend_draining_is_not_retryable() {
        assert!(!is_retryable(&EngineError::BackendDraining));
    }

    #[tokio::test]
    async fn test_backend_poisoned_maps_to_503_body() {
        let api_err: ApiError = EngineError::BackendPoisoned.into();
        let resp = api_err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("model backend temporarily unavailable"));
    }

    #[tokio::test]
    async fn test_backend_draining_maps_to_503_body() {
        let api_err: ApiError = EngineError::BackendDraining.into();
        let resp = api_err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("server shutting down"));
    }

    #[test]
    fn test_model_unavailable_is_not_retryable() {
        assert!(!is_retryable(&EngineError::ModelUnavailable(
            "budget".into()
        )));
    }

    #[tokio::test]
    async fn test_model_unavailable_maps_to_503_body() {
        let api_err: ApiError = EngineError::ModelUnavailable("budget exceeded".into()).into();
        let resp = api_err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let s = std::str::from_utf8(&body).unwrap();
        assert!(s.contains("model temporarily unavailable"));
    }
}
