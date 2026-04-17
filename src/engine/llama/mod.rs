mod inference;
mod protocol;
mod worker;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use async_trait::async_trait;

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::config::ModelConfig;
use crate::engine::backend::{InferenceBackend, ModelInfo};
use crate::engine::scheduling::ResidentSet;
use crate::error::EngineError;

use inference::LlamaAdapter;
use protocol::WorkerCommand;

pub struct LlamaBackend {
    config: ModelConfig,
    cmd_tx_slot: Mutex<Option<mpsc::Sender<WorkerCommand>>>,
    poisoned: Arc<AtomicBool>,
    draining: Arc<AtomicBool>,

    // Lazy loading + VRAM budget tracking.
    #[allow(dead_code)]
    lazy: bool,
    load_once: tokio::sync::OnceCell<()>,
    resident_set: Arc<ResidentSet>,
    estimated_vram_mb: u64,
}

impl LlamaBackend {
    pub fn new(
        config: &ModelConfig,
        resident_set: Arc<ResidentSet>,
        estimated_vram_mb: u64,
    ) -> Result<Self, EngineError> {
        if config.path.is_none() {
            return Err(EngineError::ModelLoadFailed(format!(
                "local backend '{}' has no path (config validation should have caught this)",
                config.id
            )));
        }
        Ok(Self {
            config: config.clone(),
            cmd_tx_slot: Mutex::new(None),
            poisoned: Arc::new(AtomicBool::new(false)),
            draining: Arc::new(AtomicBool::new(false)),
            lazy: config.lazy,
            load_once: tokio::sync::OnceCell::new(),
            resident_set,
            estimated_vram_mb,
        })
    }

    /// Spawn the worker OS thread and drive it through the Load command.
    async fn spawn_worker_and_send_load(&self) -> Result<(), EngineError> {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let config = self.config.clone();
        let poisoned = self.poisoned.clone();
        let thread_name = format!("flarion-llama-{}", self.config.id);

        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                worker::run(config, cmd_rx, poisoned, LlamaAdapter::default());
            })
            .map_err(|e| EngineError::ModelLoadFailed(format!("failed to spawn worker: {e}")))?;

        let (ack_tx, ack_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Load { ack: ack_tx })
            .await
            .map_err(|_| EngineError::ModelLoadFailed("worker thread exited before Load".into()))?;

        ack_rx.await.map_err(|_| {
            EngineError::ModelLoadFailed("worker thread panicked during Load".into())
        })??;

        *self
            .cmd_tx_slot
            .lock()
            .map_err(|_| EngineError::ModelLoadFailed("cmd_tx lock poisoned".into()))? =
            Some(cmd_tx);
        Ok(())
    }

    fn cmd_tx(&self) -> Result<mpsc::Sender<WorkerCommand>, EngineError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(EngineError::BackendPoisoned);
        }
        if self.draining.load(Ordering::Acquire) {
            return Err(EngineError::BackendDraining);
        }
        self.cmd_tx_slot
            .lock()
            .map_err(|_| EngineError::InferenceFailed("cmd_tx lock poisoned".into()))?
            .clone()
            .ok_or_else(|| {
                EngineError::InferenceFailed("worker not started (call load first)".into())
            })
    }

    /// Single-flight lazy load: returns immediately if already loaded,
    /// otherwise runs `try_load_inner` exactly once across concurrent callers.
    async fn ensure_loaded(&self) -> Result<(), EngineError> {
        if self.load_once.initialized() {
            return Ok(());
        }
        if self.poisoned.load(Ordering::Acquire) {
            return Err(EngineError::BackendPoisoned);
        }
        if self.draining.load(Ordering::Acquire) {
            return Err(EngineError::BackendDraining);
        }
        self.load_once
            .get_or_try_init(|| self.try_load_inner())
            .await?;
        Ok(())
    }

    /// Reserve budget → spawn worker → release on failure.
    async fn try_load_inner(&self) -> Result<(), EngineError> {
        use crate::engine::scheduling::ResidentError;

        tracing::info!(
            model_id = %self.config.id,
            estimated_mb = self.estimated_vram_mb,
            "lazy load triggered"
        );

        use std::sync::atomic::{AtomicU32, AtomicU64};
        let last_used_ms = Arc::new(AtomicU64::new(0));
        let in_flight = Arc::new(AtomicU32::new(0));
        self.resident_set
            .try_reserve(crate::engine::scheduling::ReservationRequest {
                model_id: &self.config.id,
                cost_mb: self.estimated_vram_mb,
                pinned: self.config.pin,
                last_used_ms,
                in_flight,
            })
            .map_err(|e| match e {
                ResidentError::OverBudget {
                    model_id,
                    requested_mb,
                    current_mb,
                    budget_mb,
                } => {
                    tracing::warn!(
                        %model_id,
                        requested_mb,
                        current_mb,
                        budget_mb,
                        "budget exceeded, rejecting load"
                    );
                    metrics::counter!(
                        "flarion_model_loads_total",
                        "model" => model_id.clone(),
                        "result" => "over_budget",
                    )
                    .increment(1);
                    EngineError::ModelUnavailable(format!(
                        "VRAM budget exceeded: need {requested_mb}MB, have {} free of {budget_mb}MB",
                        budget_mb.saturating_sub(current_mb)
                    ))
                }
                ResidentError::Poisoned => {
                    EngineError::InferenceFailed("resident set poisoned".into())
                }
            })?;

        // Reservation is live now; release it on any spawn/load failure below.
        match self.spawn_worker_and_send_load().await {
            Ok(()) => {
                tracing::info!(
                    model_id = %self.config.id,
                    total_mb = self.resident_set.total_reserved_mb(),
                    budget_mb = self.resident_set.budget_mb(),
                    "reserved VRAM for model"
                );
                crate::metrics::set_vram_reserved(&self.config.id, self.estimated_vram_mb);
                metrics::counter!(
                    "flarion_model_loads_total",
                    "model" => self.config.id.clone(),
                    "result" => "success",
                )
                .increment(1);
                Ok(())
            }
            Err(e) => {
                tracing::error!(
                    model_id = %self.config.id,
                    error = %e,
                    "model load failed after reservation; releasing"
                );
                self.resident_set.release(&self.config.id);
                crate::metrics::set_vram_reserved(&self.config.id, 0);
                metrics::counter!(
                    "flarion_model_loads_total",
                    "model" => self.config.id.clone(),
                    "result" => "load_failed",
                )
                .increment(1);
                Err(e)
            }
        }
    }
}

#[async_trait]
impl InferenceBackend for LlamaBackend {
    async fn load(&self) -> Result<(), EngineError> {
        self.ensure_loaded().await
    }

    async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, EngineError> {
        self.ensure_loaded().await?;
        let cmd_tx = self.cmd_tx()?;
        let (ack_tx, ack_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::Chat {
                request,
                ack: ack_tx,
            })
            .await
            .map_err(|_| EngineError::BackendPoisoned)?;
        ack_rx.await.map_err(|_| EngineError::BackendPoisoned)?
    }

    async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
        tx: mpsc::Sender<ChatCompletionChunk>,
    ) -> Result<(), EngineError> {
        self.ensure_loaded().await?;
        let cmd_tx = self.cmd_tx()?;
        let (done_tx, done_rx) = oneshot::channel();
        cmd_tx
            .send(WorkerCommand::ChatStream {
                request,
                chunks: tx,
                done: done_tx,
            })
            .await
            .map_err(|_| EngineError::BackendPoisoned)?;
        done_rx.await.map_err(|_| EngineError::BackendPoisoned)?
    }

    fn model_info(&self) -> ModelInfo {
        let loaded = !self.poisoned.load(Ordering::Acquire)
            && self
                .cmd_tx_slot
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false);
        ModelInfo {
            id: self.config.id.clone(),
            context_size: self.config.context_size,
            loaded,
            provider: "local".into(),
        }
    }

    fn provider(&self) -> &'static str {
        "local"
    }

    fn max_tokens_cap(&self) -> u32 {
        self.config.max_tokens_cap.unwrap_or(8192)
    }

    async fn shutdown(&self, grace: Duration) {
        if !self.load_once.initialized() {
            return;
        }

        self.draining.store(true, Ordering::Release);
        let cmd_tx = match self.cmd_tx_slot.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(cmd_tx) = cmd_tx else { return };

        let (ack_tx, ack_rx) = oneshot::channel();
        if cmd_tx
            .send(protocol::WorkerCommand::Shutdown { ack: ack_tx })
            .await
            .is_err()
        {
            return;
        }
        drop(cmd_tx);

        match tokio::time::timeout(grace, ack_rx).await {
            Ok(Ok(())) => info!(model_id = %self.config.id, "drained cleanly"),
            Ok(Err(_)) => warn!(
                model_id = %self.config.id,
                "worker exited without ack (panic or error)"
            ),
            Err(_) => warn!(
                model_id = %self.config.id,
                grace_secs = grace.as_secs(),
                "shutdown grace exceeded; abandoning worker"
            ),
        }

        self.resident_set.release(&self.config.id);
        crate::metrics::set_vram_reserved(&self.config.id, 0);
        tracing::info!(model_id = %self.config.id, "released reservation on shutdown");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ChatMessage;
    use crate::config::BackendType;
    use std::path::PathBuf;

    fn test_config() -> ModelConfig {
        ModelConfig {
            id: "test".into(),
            backend: BackendType::Local,
            path: Some(PathBuf::from("/tmp/x.gguf")),
            context_size: 4096,
            gpu_layers: 0,
            threads: None,
            batch_size: None,
            seed: None,
            api_key: None,
            base_url: None,
            upstream_model: None,
            timeout_secs: None,
            max_tokens_cap: None,
            lazy: false,
            vram_mb: None,
            pin: false,
        }
    }

    fn test_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "test".into(),
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
    async fn backend_rejects_while_poisoned() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
        )
        .unwrap();
        backend.poisoned.store(true, Ordering::Release);

        let err = backend.chat_completion(test_request()).await.unwrap_err();
        assert!(matches!(err, EngineError::BackendPoisoned));
    }

    #[tokio::test]
    async fn backend_rejects_while_draining() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
        )
        .unwrap();
        backend.draining.store(true, Ordering::Release);

        let err = backend.chat_completion(test_request()).await.unwrap_err();
        assert!(matches!(err, EngineError::BackendDraining));
    }

    // chat_completion always calls ensure_loaded, which fails fast here
    // because /tmp/x.gguf doesn't exist — so we see ModelLoadFailed.
    #[tokio::test]
    async fn backend_without_load_returns_error() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
        )
        .unwrap();
        let err = backend.chat_completion(test_request()).await.unwrap_err();
        assert!(
            matches!(err, EngineError::ModelLoadFailed(_)),
            "expected ModelLoadFailed, got {err:?}"
        );
    }

    #[test]
    fn backend_new_rejects_missing_path() {
        let mut cfg = test_config();
        cfg.path = None;
        match LlamaBackend::new(&cfg, crate::engine::scheduling::ResidentSet::new(0), 0) {
            Err(EngineError::ModelLoadFailed(_)) => {}
            Err(other) => panic!("expected ModelLoadFailed, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn backend_model_info_reflects_poisoned() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
        )
        .unwrap();
        assert!(!backend.model_info().loaded);
        backend.poisoned.store(true, Ordering::Release);
        assert!(!backend.model_info().loaded);
    }

    #[test]
    fn lazy_backend_does_not_load_on_new() {
        let mut cfg = test_config();
        cfg.lazy = true;
        let resident_set = crate::engine::scheduling::ResidentSet::new(0);
        let backend = LlamaBackend::new(&cfg, resident_set, 0).unwrap();
        assert!(!backend.load_once.initialized());
        assert!(backend.cmd_tx_slot.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn ensure_loaded_returns_over_budget_when_resident_set_full() {
        let cfg = test_config();
        let resident_set = crate::engine::scheduling::ResidentSet::new(1000);
        use std::sync::atomic::{AtomicU32, AtomicU64};
        resident_set
            .try_reserve(crate::engine::scheduling::ReservationRequest {
                model_id: "other",
                cost_mb: 1000,
                pinned: false,
                last_used_ms: std::sync::Arc::new(AtomicU64::new(0)),
                in_flight: std::sync::Arc::new(AtomicU32::new(0)),
            })
            .unwrap();
        let backend = LlamaBackend::new(&cfg, resident_set, 500).unwrap();
        let err = backend.ensure_loaded().await.unwrap_err();
        assert!(matches!(err, EngineError::ModelUnavailable(_)));
        // OnceCell must stay uninitialized so a later retry can succeed, and
        // no worker should have been spawned before the reservation failed.
        assert!(!backend.load_once.initialized());
        assert!(backend.cmd_tx_slot.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn ensure_loaded_returns_draining_error() {
        let cfg = test_config();
        let resident_set = crate::engine::scheduling::ResidentSet::new(0);
        let backend = LlamaBackend::new(&cfg, resident_set, 0).unwrap();
        backend.draining.store(true, Ordering::Release);
        let err = backend.ensure_loaded().await.unwrap_err();
        assert!(matches!(err, EngineError::BackendDraining));
    }

    #[tokio::test]
    async fn ensure_loaded_returns_poisoned_error() {
        let cfg = test_config();
        let resident_set = crate::engine::scheduling::ResidentSet::new(0);
        let backend = LlamaBackend::new(&cfg, resident_set, 0).unwrap();
        backend.poisoned.store(true, Ordering::Release);
        let err = backend.ensure_loaded().await.unwrap_err();
        assert!(matches!(err, EngineError::BackendPoisoned));
    }
}
