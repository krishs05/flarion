use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest};
use crate::engine::backend::InferenceBackend;
use crate::error::ApiError;

pub async fn chat_completions(
    State(backend): State<Arc<dyn InferenceBackend>>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, ApiError> {
    // Validate model matches
    let info = backend.model_info();
    if request.model != info.id {
        return Err(ApiError::ModelNotFound(request.model.clone()));
    }

    if request.messages.is_empty() {
        return Err(ApiError::BadRequest("messages cannot be empty".to_string()));
    }

    if request.stream {
        let (tx, rx) = tokio::sync::mpsc::channel::<ChatCompletionChunk>(256);
        let backend_clone = backend.clone();
        let request_clone = request.clone();

        tokio::spawn(async move {
            if let Err(e) = backend_clone
                .chat_completion_stream(request_clone, tx)
                .await
            {
                tracing::error!("streaming inference failed: {e}");
            }
        });

        let stream = ReceiverStream::new(rx).map(|chunk| {
            let data = serde_json::to_string(&chunk).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().data(data))
        });

        // Append [DONE] sentinel after the chunk stream ends
        let done_stream = futures_util::stream::once(async {
            Ok::<_, Infallible>(Event::default().data("[DONE]"))
        });

        let combined = stream.chain(done_stream);

        Ok(Sse::new(combined)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let response = backend
            .chat_completion(request)
            .await
            .map_err(ApiError::from)?;
        Ok(Json(response).into_response())
    }
}
