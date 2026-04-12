use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::error::EngineError;

pub struct ModelInfo {
    pub id: String,
    pub context_size: u32,
    pub loaded: bool,
}

#[async_trait::async_trait]
pub trait InferenceBackend: Send + Sync {
    async fn load(&self) -> Result<(), EngineError>;
    async fn chat_completion(&self, request: ChatCompletionRequest) -> Result<ChatCompletionResponse, EngineError>;
    async fn chat_completion_stream(&self, request: ChatCompletionRequest, tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>) -> Result<(), EngineError>;
    fn model_info(&self) -> ModelInfo;
}
