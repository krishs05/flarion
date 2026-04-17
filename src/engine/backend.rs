use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::error::EngineError;

#[allow(dead_code)]
pub struct ModelInfo {
    pub id: String,
    pub context_size: u32,
    pub loaded: bool,
    pub provider: String,
}

#[async_trait::async_trait]
pub trait InferenceBackend: Send + Sync {
    async fn load(&self) -> Result<(), EngineError>;
    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError>;
    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
        tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError>;
    fn model_info(&self) -> ModelInfo;

    /// Provider identifier used in /v1/models `owned_by` field and in logs.
    /// Returns one of: "local", "openai", "groq", "anthropic".
    #[allow(dead_code)]
    fn provider(&self) -> &'static str;

    /// Upper bound on `max_tokens` this backend will accept. Requests above
    /// this are silently clamped in the chat handler before inference runs.
    fn max_tokens_cap(&self) -> u32 {
        8192
    }

    /// Gracefully shut down this backend, cancelling any in-flight work
    /// within `grace`. Default is a no-op — only backends that own
    /// non-async resources (e.g. OS-thread workers) need to override.
    async fn shutdown(&self, _grace: std::time::Duration) {}
}
