use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("failed to load model: {0}")]
    ModelLoadFailed(String),
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("context length exceeded: requested {requested}, max {max}")]
    ContextLengthExceeded { requested: usize, max: usize },
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("model not found: {0}")]
    ModelNotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<EngineError> for ApiError {
    fn from(err: EngineError) -> Self {
        match err {
            EngineError::ModelNotFound(id) => ApiError::ModelNotFound(id),
            EngineError::ModelLoadFailed(msg) => ApiError::Internal(msg),
            EngineError::InferenceFailed(msg) => ApiError::Internal(msg),
            EngineError::ContextLengthExceeded { requested, max } => {
                ApiError::BadRequest(format!(
                    "context length exceeded: requested {requested}, max {max}"
                ))
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
            ApiError::ModelNotFound(_) => (
                StatusCode::NOT_FOUND,
                "invalid_request_error",
                "model_not_found",
            ),
            ApiError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "internal_error",
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
        let err = ApiError::ModelNotFound("nonexistent".into());
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
        assert!(matches!(api_err, ApiError::ModelNotFound(_)));

        let engine_err = EngineError::ContextLengthExceeded {
            requested: 8000,
            max: 4096,
        };
        let api_err: ApiError = engine_err.into();
        assert!(matches!(api_err, ApiError::BadRequest(_)));
    }
}
