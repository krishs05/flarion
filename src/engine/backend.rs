use std::sync::Weak;

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

    /// Unload the model and release its VRAM reservation. Returns
    /// `EngineError::BackendBusy` if an in-flight request prevents unload.
    /// Default no-op for backends without resident resources (cloud).
    async fn unload(&self) -> Result<(), EngineError> {
        Ok(())
    }

    /// Set or clear the session pin for this backend. Default no-op is correct
    /// for cloud/remote backends (nothing to pin); local backends can override
    /// to flip a runtime `AtomicBool` that `LoadCoordinator` checks before
    /// evicting.
    async fn pin(&self, _pinned: bool) -> Result<(), EngineError> {
        Ok(())
    }

    /// Install a weak handle to the eviction orchestrator. Called after
    /// the registry is built (late-bind to break the Registry↔backend
    /// cycle). Default no-op for backends that don't drive eviction
    /// (cloud).
    async fn bind_evictor(&self, _evictor: Weak<dyn Evictor>) {}
}

/// Eviction orchestrator handed to each local backend after registry
/// construction. A backend that needs to load a new model over budget calls
/// `evictor.unload(victim_id)` to free space.
#[async_trait::async_trait]
pub trait Evictor: Send + Sync {
    async fn unload(&self, model_id: &str) -> Result<(), EngineError>;
}
