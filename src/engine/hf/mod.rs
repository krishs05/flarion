//! HF/safetensors inference backend. Loads Hugging Face-native models
//! (safetensors + config.json + tokenizer.json) via Candle. Peer to the
//! llama.cpp backend; both remain first-class.
//!
//! Wave 1 scope: type skeleton only — all inference entry points return
//! `EngineError::NotImplemented`. See
//! `docs/superpowers/specs/2026-04-18-hf-safetensors-backend-design.md`.

use std::sync::Weak;

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::config::ModelConfig;
use crate::engine::backend::{Evictor, InferenceBackend, ModelInfo};
use crate::error::EngineError;

/// HF/safetensors backend. Wave 1 is a stub — `load` and inference entry
/// points return `EngineError::NotImplemented`. Wave 2 lands the loader
/// and tokenizer; Wave 3 lands the first architecture.
pub struct HfBackend {
    id: String,
    context_size: u32,
    max_tokens_cap: u32,
}

impl HfBackend {
    /// Construct an `HfBackend` from a validated model config. Does not load
    /// weights. This is infallible in Wave 1 but returns `Result` so the
    /// signature survives Wave 2 (where it parses config.json on disk).
    pub fn new(cfg: &ModelConfig) -> Result<Self, EngineError> {
        debug_assert_eq!(cfg.backend, crate::config::BackendType::Hf);
        Ok(Self {
            id: cfg.id.clone(),
            context_size: cfg.context_size,
            max_tokens_cap: cfg.max_tokens_cap.unwrap_or(8192),
        })
    }
}

#[async_trait::async_trait]
impl InferenceBackend for HfBackend {
    async fn load(&self) -> Result<(), EngineError> {
        Err(EngineError::NotImplemented(format!(
            "hf backend load for '{}': Wave 2 lands the loader",
            self.id
        )))
    }

    async fn chat_completion(
        &self,
        _request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        Err(EngineError::NotImplemented(format!(
            "hf backend chat_completion for '{}'",
            self.id
        )))
    }

    async fn chat_completion_stream(
        &self,
        _request: ChatCompletionRequest,
        _tx: tokio::sync::mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        Err(EngineError::NotImplemented(format!(
            "hf backend chat_completion_stream for '{}'",
            self.id
        )))
    }

    fn model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            context_size: self.context_size,
            loaded: false,
            provider: "hf".into(),
        }
    }

    fn provider(&self) -> &'static str {
        "hf"
    }

    fn max_tokens_cap(&self) -> u32 {
        self.max_tokens_cap
    }

    async fn bind_evictor(&self, _evictor: Weak<dyn Evictor>) {
        // Wave 8 wires eviction. Wave 1 no-ops.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::backend::InferenceBackend;

    fn hf_cfg() -> crate::config::ModelConfig {
        crate::config::ModelConfig {
            id: "test-hf".into(),
            backend: crate::config::BackendType::Hf,
            path: None,
            context_size: 4096,
            gpu_layers: 99,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: None,
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: true,
            vram_mb: None,
            pin: false,
            gpus: Vec::new(),
            repo: Some("org/model".into()),
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
        }
    }

    #[test]
    fn test_hf_backend_constructs_from_config() {
        let cfg = hf_cfg();
        let backend = HfBackend::new(&cfg).expect("construction should succeed");
        assert_eq!(backend.model_info().id, "test-hf");
        assert_eq!(backend.provider(), "hf");
        assert!(!backend.model_info().loaded);
    }

    #[tokio::test]
    async fn test_hf_backend_load_returns_not_implemented() {
        let cfg = hf_cfg();
        let backend = HfBackend::new(&cfg).unwrap();
        let err = backend.load().await.unwrap_err();
        assert!(
            matches!(err, crate::error::EngineError::NotImplemented(_)),
            "got: {err:?}"
        );
    }

    #[tokio::test]
    async fn test_hf_backend_chat_completion_returns_not_implemented() {
        use crate::api::types::{ChatCompletionRequest, ChatMessage};
        let cfg = hf_cfg();
        let backend = HfBackend::new(&cfg).unwrap();
        let req = ChatCompletionRequest {
            model: "test-hf".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            max_tokens: 16,
            temperature: 0.7,
            top_p: 0.9,
            stream: false,
            stop: Vec::new(),
            seed: None,
        };
        let err = backend.chat_completion(req).await.unwrap_err();
        assert!(matches!(err, crate::error::EngineError::NotImplemented(_)));
    }
}
