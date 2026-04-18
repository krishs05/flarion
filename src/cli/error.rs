use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("cannot reach {url}: {source}")]
    Unreachable {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found: {resource}")]
    NotFound { resource: String },
    #[error("conflict: {reason}")]
    Conflict { reason: String },
    #[error("request timed out")]
    Timeout,
    #[error("server error {status}: {body}")]
    Server { status: u16, body: String },
    #[error("decode error: {reason}")]
    Decode { reason: String },
    #[error("stream error: {reason}")]
    Stream { reason: String },
}
