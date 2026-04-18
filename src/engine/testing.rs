#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::api::types::{
    ChatCompletionChoice, ChatCompletionChunk, ChatCompletionChunkChoice, ChatCompletionRequest,
    ChatCompletionResponse, ChatDelta, ChatMessage, Usage,
};
use crate::engine::backend::{InferenceBackend, ModelInfo};
use crate::error::EngineError;

/// Describes how a MockBackend should behave for each call.
#[derive(Clone)]
pub enum MockBehavior {
    /// Return a fixed ChatCompletionResponse.
    Succeed { response: ChatCompletionResponse },
    /// Return a fixed EngineError (clone from template).
    Fail { template: Arc<EngineError> },
    /// Sleep for `delay` before returning Err(Timeout).
    Timeout { delay: Duration },
    /// Stream the given chunks then terminate cleanly.
    StreamChunks { chunks: Vec<String> },
    /// Stream the first N chunks then fail with the given error.
    StreamThenError {
        chunks: Vec<String>,
        err: Arc<EngineError>,
    },
    /// Stream chunks with a delay between each, simulating progressive generation.
    StreamPaced {
        chunks: Vec<String>,
        per_chunk_delay: Duration,
    },
}

pub struct MockBackend {
    id: String,
    provider: &'static str,
    behaviors: Mutex<Vec<MockBehavior>>,
    call_count: AtomicUsize,
    max_tokens_cap_override: Option<u32>,
    cancel_observed: Arc<AtomicBool>,
}

impl MockBackend {
    pub fn new(id: impl Into<String>, behavior: MockBehavior) -> Self {
        Self {
            id: id.into(),
            provider: "mock",
            behaviors: Mutex::new(vec![behavior]),
            call_count: AtomicUsize::new(0),
            max_tokens_cap_override: None,
            cancel_observed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Queue multiple behaviors — one used per call, in order. The last repeats if exhausted.
    #[allow(dead_code)]
    pub fn with_behaviors(id: impl Into<String>, behaviors: Vec<MockBehavior>) -> Self {
        Self {
            id: id.into(),
            provider: "mock",
            behaviors: Mutex::new(behaviors),
            call_count: AtomicUsize::new(0),
            max_tokens_cap_override: None,
            cancel_observed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Override the max_tokens cap. Call before wrapping the mock in `Arc`.
    pub fn with_max_tokens_cap(mut self, cap: u32) -> Self {
        self.max_tokens_cap_override = Some(cap);
        self
    }

    pub fn succeeding(id: &str, text: &str) -> Self {
        Self::new(
            id,
            MockBehavior::Succeed {
                response: ChatCompletionResponse {
                    id: format!("chatcmpl-mock-{id}"),
                    object: "chat.completion".into(),
                    created: 0,
                    model: id.to_string(),
                    choices: vec![ChatCompletionChoice {
                        index: 0,
                        message: ChatMessage {
                            role: "assistant".into(),
                            content: text.to_string(),
                        },
                        finish_reason: "stop".into(),
                    }],
                    usage: Usage {
                        prompt_tokens: 1,
                        completion_tokens: 1,
                        total_tokens: 2,
                    },
                },
            },
        )
    }

    pub fn failing(id: &str, err: EngineError) -> Self {
        Self::new(
            id,
            MockBehavior::Fail {
                template: Arc::new(err),
            },
        )
    }

    pub fn timing_out(id: &str, delay: Duration) -> Self {
        Self::new(id, MockBehavior::Timeout { delay })
    }

    pub fn streaming_chunks(id: &str, chunks: Vec<String>) -> Self {
        Self::new(id, MockBehavior::StreamChunks { chunks })
    }

    pub fn streaming_paced(id: &str, chunks: Vec<String>, per_chunk_delay: Duration) -> Self {
        Self::new(
            id,
            MockBehavior::StreamPaced {
                chunks,
                per_chunk_delay,
            },
        )
    }

    pub fn cancel_observed(&self) -> bool {
        self.cancel_observed.load(Ordering::SeqCst)
    }

    pub fn streaming_then_error(id: &str, chunks: Vec<String>, err: EngineError) -> Self {
        Self::new(
            id,
            MockBehavior::StreamThenError {
                chunks,
                err: Arc::new(err),
            },
        )
    }

    #[allow(dead_code)]
    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    fn next_behavior(&self) -> MockBehavior {
        let behaviors = self.behaviors.lock().unwrap();
        let idx = self
            .call_count
            .fetch_add(1, Ordering::SeqCst)
            .min(behaviors.len().saturating_sub(1));
        behaviors[idx].clone()
    }

    fn clone_err(template: &Arc<EngineError>) -> EngineError {
        match template.as_ref() {
            EngineError::Timeout => EngineError::Timeout,
            EngineError::Network(msg) => EngineError::Network(msg.clone()),
            EngineError::InferenceFailed(msg) => EngineError::InferenceFailed(msg.clone()),
            EngineError::RateLimited { retry_after } => EngineError::RateLimited {
                retry_after: *retry_after,
            },
            EngineError::UpstreamServerError { status, body } => EngineError::UpstreamServerError {
                status: *status,
                body: body.clone(),
            },
            EngineError::ModelLoadFailed(msg) => EngineError::ModelLoadFailed(msg.clone()),
            EngineError::ModelNotFound(msg) => EngineError::ModelNotFound(msg.clone()),
            EngineError::ContextLengthExceeded { requested, max } => {
                EngineError::ContextLengthExceeded {
                    requested: *requested,
                    max: *max,
                }
            }
            EngineError::AllBackendsFailed { route_id, .. } => EngineError::InferenceFailed(
                format!("cloned AllBackendsFailed for route '{route_id}' (lossy)"),
            ),
            EngineError::BackendPoisoned => EngineError::BackendPoisoned,
            EngineError::BackendDraining => EngineError::BackendDraining,
            EngineError::ModelUnavailable(msg) => EngineError::ModelUnavailable(msg.clone()),
            EngineError::BackendBusy => EngineError::BackendBusy,
            EngineError::NotImplemented(msg) => EngineError::NotImplemented(msg.clone()),
        }
    }
}

#[async_trait]
impl InferenceBackend for MockBackend {
    async fn load(&self) -> Result<(), EngineError> {
        Ok(())
    }

    async fn chat_completion(
        &self,
        _request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        match self.next_behavior() {
            MockBehavior::Succeed { response } => Ok(response),
            MockBehavior::Fail { template } => Err(Self::clone_err(&template)),
            MockBehavior::Timeout { delay } => {
                tokio::time::sleep(delay).await;
                Err(EngineError::Timeout)
            }
            MockBehavior::StreamChunks { .. }
            | MockBehavior::StreamThenError { .. }
            | MockBehavior::StreamPaced { .. } => Err(EngineError::InferenceFailed(
                "streaming behavior invoked on non-streaming path".into(),
            )),
        }
    }

    async fn chat_completion_stream(
        &self,
        _request: ChatCompletionRequest,
        tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        match self.next_behavior() {
            MockBehavior::Succeed { response } => {
                let content = response
                    .choices
                    .first()
                    .map(|c| c.message.content.clone())
                    .unwrap_or_default();
                let _ = tx
                    .send(chunk(&response.id, &response.model, Some(&content), None))
                    .await;
                Ok(())
            }
            MockBehavior::Fail { template } => Err(Self::clone_err(&template)),
            MockBehavior::Timeout { delay } => {
                tokio::time::sleep(delay).await;
                Err(EngineError::Timeout)
            }
            MockBehavior::StreamChunks { chunks } => {
                let id = format!("chatcmpl-mock-{}", self.id);
                for text in chunks {
                    if tx
                        .send(chunk(&id, &self.id, Some(&text), None))
                        .await
                        .is_err()
                    {
                        self.cancel_observed.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                }
                let _ = tx.send(chunk(&id, &self.id, None, Some("stop"))).await;
                Ok(())
            }
            MockBehavior::StreamThenError { chunks, err } => {
                let id = format!("chatcmpl-mock-{}", self.id);
                for text in chunks {
                    if tx
                        .send(chunk(&id, &self.id, Some(&text), None))
                        .await
                        .is_err()
                    {
                        self.cancel_observed.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                }
                Err(Self::clone_err(&err))
            }
            MockBehavior::StreamPaced {
                chunks,
                per_chunk_delay,
            } => {
                let id = format!("chatcmpl-mock-{}", self.id);
                for text in chunks {
                    tokio::time::sleep(per_chunk_delay).await;
                    if tx
                        .send(chunk(&id, &self.id, Some(&text), None))
                        .await
                        .is_err()
                    {
                        self.cancel_observed.store(true, Ordering::SeqCst);
                        return Ok(());
                    }
                }
                let _ = tx.send(chunk(&id, &self.id, None, Some("stop"))).await;
                Ok(())
            }
        }
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            context_size: 4096,
            loaded: true,
            provider: self.provider.to_string(),
        }
    }

    fn provider(&self) -> &'static str {
        self.provider
    }

    fn max_tokens_cap(&self) -> u32 {
        self.max_tokens_cap_override.unwrap_or(8192)
    }
}

fn chunk(
    id: &str,
    model: &str,
    content: Option<&str>,
    finish: Option<&str>,
) -> ChatCompletionChunk {
    ChatCompletionChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".into(),
        created: 0,
        model: model.to_string(),
        choices: vec![ChatCompletionChunkChoice {
            index: 0,
            delta: ChatDelta {
                role: None,
                content: content.map(String::from),
            },
            finish_reason: finish.map(String::from),
        }],
    }
}

#[cfg(test)]
mod self_tests {
    use super::*;
    use crate::api::types::{ChatCompletionRequest, ChatMessage};

    fn minimal_request(model: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            stream: false,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        }
    }

    #[tokio::test]
    async fn succeeding_returns_response() {
        let backend = MockBackend::succeeding("m1", "hello");
        let resp = backend
            .chat_completion(minimal_request("m1"))
            .await
            .unwrap();
        assert_eq!(resp.choices[0].message.content, "hello");
    }

    #[tokio::test]
    async fn failing_returns_error() {
        let backend = MockBackend::failing("m1", EngineError::Network("down".into()));
        let err = backend
            .chat_completion(minimal_request("m1"))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Network(_)));
    }

    #[tokio::test]
    async fn timing_out_returns_timeout_after_delay() {
        let backend = MockBackend::timing_out("m1", Duration::from_millis(10));
        let err = backend
            .chat_completion(minimal_request("m1"))
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Timeout));
    }

    #[tokio::test]
    async fn streaming_chunks_emits_all_chunks() {
        let backend = MockBackend::streaming_chunks("m1", vec!["foo".into(), "bar".into()]);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ChatCompletionChunk>(8);
        let req = minimal_request("m1");
        tokio::spawn(async move {
            backend.chat_completion_stream(req, tx).await.unwrap();
        });
        let mut texts = Vec::new();
        while let Some(chunk) = rx.recv().await {
            if let Some(c) = chunk.choices[0].delta.content.clone() {
                texts.push(c);
            }
        }
        assert_eq!(texts, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[tokio::test]
    async fn streaming_then_error_emits_chunks_then_errors() {
        let backend = Arc::new(MockBackend::streaming_then_error(
            "m1",
            vec!["a".into(), "b".into()],
            EngineError::Network("broke".into()),
        ));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ChatCompletionChunk>(8);
        let backend_clone = backend.clone();
        let req = minimal_request("m1");
        let handle =
            tokio::spawn(async move { backend_clone.chat_completion_stream(req, tx).await });
        let mut received = 0;
        while let Some(_chunk) = rx.recv().await {
            received += 1;
        }
        assert_eq!(received, 2);
        let result = handle.await.unwrap();
        assert!(matches!(result, Err(EngineError::Network(_))));
    }
}
