mod inference;
mod protocol;
mod worker;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, Mutex, Notify, OnceCell};
use tracing::{info, warn};

use crate::api::types::{ChatCompletionChunk, ChatCompletionRequest, ChatCompletionResponse};
use crate::config::ModelConfig;
use crate::engine::backend::{Evictor, InferenceBackend, ModelInfo};
use crate::engine::scheduling::{ReservationRequest, ResidentSet};
use crate::error::EngineError;

use inference::LlamaAdapter;
use protocol::WorkerCommand;

pub(crate) struct LoadedState {
    cmd_tx: mpsc::Sender<WorkerCommand>,
    /// Handle to the worker OS thread; joined on shutdown / unload.
    worker_handle: Option<JoinHandle<()>>,
}

pub(crate) enum LoadState {
    Unloaded,
    Loading(Arc<Notify>),
    Loaded(LoadedState),
}

pub struct LlamaBackend {
    config: ModelConfig,
    poisoned: Arc<AtomicBool>,
    draining: Arc<AtomicBool>,

    resident_set: Arc<ResidentSet>,
    estimated_vram_mb: u64,

    /// Primary state machine (replaces 2F's OnceCell + cmd_tx_slot).
    pub(crate) load_state: Mutex<LoadState>,
    /// Unix millis of most recent request completion or load success.
    pub(crate) last_used_ms: Arc<AtomicU64>,
    /// Count of requests currently executing + loading sentinel.
    pub(crate) in_flight: Arc<AtomicU32>,
    /// Process-wide mutex serializing load + evict sequences.
    #[allow(dead_code)]
    load_coordinator: Arc<Mutex<()>>,
    /// Late-bound (set after registry construction).
    #[allow(dead_code)]
    evictor: OnceCell<Weak<dyn Evictor>>,
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl LlamaBackend {
    pub fn new(
        config: &ModelConfig,
        resident_set: Arc<ResidentSet>,
        estimated_vram_mb: u64,
        load_coordinator: Arc<Mutex<()>>,
    ) -> Result<Self, EngineError> {
        if config.path.is_none() {
            return Err(EngineError::ModelLoadFailed(format!(
                "local backend '{}' has no path (config validation should have caught this)",
                config.id
            )));
        }
        Ok(Self {
            config: config.clone(),
            poisoned: Arc::new(AtomicBool::new(false)),
            draining: Arc::new(AtomicBool::new(false)),
            resident_set,
            estimated_vram_mb,
            load_state: Mutex::new(LoadState::Unloaded),
            last_used_ms: Arc::new(AtomicU64::new(0)),
            in_flight: Arc::new(AtomicU32::new(0)),
            load_coordinator,
            evictor: OnceCell::new(),
        })
    }

    pub(crate) fn touch_last_used(&self) {
        self.last_used_ms.store(now_unix_ms(), Ordering::Release);
    }

    /// Spawn the worker OS thread and drive it through Load. Returns the
    /// populated `LoadedState` on success. Task 14 adds the eviction loop
    /// around this call.
    async fn spawn_worker_and_send_load(&self) -> Result<LoadedState, EngineError> {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let config = self.config.clone();
        let poisoned = self.poisoned.clone();
        let thread_name = format!("flarion-llama-{}", self.config.id);

        let handle = std::thread::Builder::new()
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

        Ok(LoadedState {
            cmd_tx,
            worker_handle: Some(handle),
        })
    }

    /// Single-flight 2F-compatible ensure_loaded: returns Ok when Loaded,
    /// otherwise transitions Unloaded → Loading → Loaded. No eviction yet
    /// (Task 14 adds that to load_as_leader).
    pub(crate) async fn ensure_loaded(&self) -> Result<(), EngineError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(EngineError::BackendPoisoned);
        }
        if self.draining.load(Ordering::Acquire) {
            return Err(EngineError::BackendDraining);
        }

        loop {
            let notify = {
                let mut guard = self.load_state.lock().await;
                match &*guard {
                    LoadState::Loaded(_) => return Ok(()),
                    LoadState::Loading(n) => n.clone(),
                    LoadState::Unloaded => {
                        let n = Arc::new(Notify::new());
                        *guard = LoadState::Loading(n.clone());
                        drop(guard);
                        return self.load_as_leader(n).await;
                    }
                }
            };
            notify.notified().await;
        }
    }

    async fn load_as_leader(&self, notify: Arc<Notify>) -> Result<(), EngineError> {
        let _coord = self.load_coordinator.lock().await;

        tracing::info!(
            model_id = %self.config.id,
            estimated_mb = self.estimated_vram_mb,
            "lazy load triggered"
        );

        let req = ReservationRequest {
            model_id: &self.config.id,
            cost_mb: self.estimated_vram_mb,
            pinned: self.config.pin,
            last_used_ms: self.last_used_ms.clone(),
            in_flight: self.in_flight.clone(),
        };

        if let Err(e) = self.resident_set.try_reserve(req) {
            use crate::engine::scheduling::ResidentError;
            self.fail_loading(&notify).await;
            return Err(match e {
                ResidentError::OverBudget {
                    requested_mb,
                    current_mb,
                    budget_mb,
                    ..
                } => {
                    metrics::counter!(
                        "flarion_model_loads_total",
                        "model" => self.config.id.clone(),
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
            });
        }

        match self.spawn_worker_and_send_load().await {
            Ok(loaded) => {
                {
                    let mut guard = self.load_state.lock().await;
                    *guard = LoadState::Loaded(loaded);
                }
                notify.notify_waiters();
                self.touch_last_used();
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
                self.resident_set.release(&self.config.id);
                crate::metrics::set_vram_reserved(&self.config.id, 0);
                self.fail_loading(&notify).await;
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

    async fn fail_loading(&self, notify: &Notify) {
        let mut guard = self.load_state.lock().await;
        *guard = LoadState::Unloaded;
        notify.notify_waiters();
    }

    /// Request-path entry point. Returns an RAII guard that keeps
    /// `in_flight > 0` for the duration of the request, preventing the
    /// model from being chosen as an eviction victim while in use.
    /// Acquires `in_flight` under `load_state` lock for atomicity with
    /// `unload`'s busy check (Task 13).
    pub(crate) async fn ensure_loaded_for_request(
        &self,
    ) -> Result<InFlightGuard, EngineError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(EngineError::BackendPoisoned);
        }
        if self.draining.load(Ordering::Acquire) {
            return Err(EngineError::BackendDraining);
        }

        loop {
            let notify = {
                let mut guard = self.load_state.lock().await;
                match &*guard {
                    LoadState::Loaded(_) => {
                        // Bump in_flight while still holding the state lock
                        // so unload's in_flight check (Task 13) can't race.
                        self.in_flight.fetch_add(1, Ordering::Release);
                        self.touch_last_used();
                        return Ok(InFlightGuard::new(self.in_flight.clone()));
                    }
                    LoadState::Loading(n) => n.clone(),
                    LoadState::Unloaded => {
                        let n = Arc::new(Notify::new());
                        *guard = LoadState::Loading(n.clone());
                        drop(guard);
                        return self.load_as_leader_for_request(n).await;
                    }
                }
            };
            notify.notified().await;
        }
    }

    /// Variant of `load_as_leader` that returns an `InFlightGuard` on success
    /// (so the request caller observes `in_flight >= 1` continuously from
    /// the moment the load begins until the request drops its guard).
    async fn load_as_leader_for_request(
        &self,
        notify: Arc<Notify>,
    ) -> Result<InFlightGuard, EngineError> {
        // Loading sentinel: keep in_flight > 0 while the load is in progress
        // so pick_eviction_candidates won't pick this not-yet-loaded model.
        self.in_flight.fetch_add(1, Ordering::Release);
        let sentinel = InFlightGuard::new(self.in_flight.clone());

        match self.load_as_leader(notify).await {
            Ok(()) => {
                // Transfer ownership: forget the sentinel (avoiding its Drop
                // decrement) and hand the caller a fresh guard. Net effect:
                // in_flight stays at +1, caller's guard decrements on drop.
                std::mem::forget(sentinel);
                Ok(InFlightGuard::new(self.in_flight.clone()))
            }
            Err(e) => {
                drop(sentinel);
                Err(e)
            }
        }
    }

    /// Snapshot helper for chat_completion paths: clones the current cmd_tx
    /// if Loaded. Does NOT bump `in_flight` (Task 12 adds a request-path
    /// wrapper that does so atomically).
    async fn cmd_tx(&self) -> Result<mpsc::Sender<WorkerCommand>, EngineError> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(EngineError::BackendPoisoned);
        }
        if self.draining.load(Ordering::Acquire) {
            return Err(EngineError::BackendDraining);
        }
        let guard = self.load_state.lock().await;
        match &*guard {
            LoadState::Loaded(loaded) => Ok(loaded.cmd_tx.clone()),
            LoadState::Loading(_) | LoadState::Unloaded => Err(EngineError::InferenceFailed(
                "worker not ready (ensure_loaded should run first)".into(),
            )),
        }
    }
}

pub(crate) struct InFlightGuard {
    counter: Arc<AtomicU32>,
}

impl InFlightGuard {
    fn new(counter: Arc<AtomicU32>) -> Self {
        Self { counter }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Release);
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
        let _guard = self.ensure_loaded_for_request().await?;
        let cmd_tx = self.cmd_tx().await?;
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
        let _guard = self.ensure_loaded_for_request().await?;
        let cmd_tx = self.cmd_tx().await?;
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
        let loaded = !self.poisoned.load(Ordering::Acquire) && {
            // try_lock keeps model_info non-blocking.
            match self.load_state.try_lock() {
                Ok(g) => matches!(&*g, LoadState::Loaded(_)),
                Err(_) => false, // contended; report not-loaded
            }
        };
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
        self.draining.store(true, Ordering::Release);
        let loaded = {
            let mut guard = self.load_state.lock().await;
            match std::mem::replace(&mut *guard, LoadState::Unloaded) {
                LoadState::Loaded(l) => Some(l),
                _ => None,
            }
        };
        let Some(mut loaded) = loaded else { return };

        let (ack_tx, ack_rx) = oneshot::channel();
        if loaded
            .cmd_tx
            .send(WorkerCommand::Shutdown { ack: ack_tx })
            .await
            .is_err()
        {
            return;
        }
        drop(loaded.cmd_tx);

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

        if let Some(handle) = loaded.worker_handle.take() {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
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
            Arc::new(tokio::sync::Mutex::new(())),
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
            Arc::new(tokio::sync::Mutex::new(())),
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
            Arc::new(tokio::sync::Mutex::new(())),
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
        match LlamaBackend::new(
            &cfg,
            crate::engine::scheduling::ResidentSet::new(0),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        ) {
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
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        assert!(!backend.model_info().loaded);
        backend.poisoned.store(true, Ordering::Release);
        assert!(!backend.model_info().loaded);
    }

    #[tokio::test]
    async fn lazy_backend_does_not_load_on_new() {
        let mut cfg = test_config();
        cfg.lazy = true;
        let resident_set = crate::engine::scheduling::ResidentSet::new(0);
        let backend = LlamaBackend::new(
            &cfg,
            resident_set,
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let state = backend.load_state.lock().await;
        assert!(matches!(&*state, LoadState::Unloaded));
    }

    #[tokio::test]
    async fn ensure_loaded_returns_over_budget_when_resident_set_full() {
        let cfg = test_config();
        let resident_set = crate::engine::scheduling::ResidentSet::new(1000);
        resident_set
            .try_reserve(crate::engine::scheduling::ReservationRequest {
                model_id: "other",
                cost_mb: 1000,
                pinned: false,
                last_used_ms: std::sync::Arc::new(AtomicU64::new(0)),
                in_flight: std::sync::Arc::new(AtomicU32::new(0)),
            })
            .unwrap();
        let backend = LlamaBackend::new(
            &cfg,
            resident_set,
            500,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let err = backend.ensure_loaded().await.unwrap_err();
        assert!(matches!(err, EngineError::ModelUnavailable(_)));
        // State must return to Unloaded so a later retry can succeed, and
        // no worker should have been spawned before the reservation failed.
        let state = backend.load_state.lock().await;
        assert!(matches!(&*state, LoadState::Unloaded));
    }

    #[tokio::test]
    async fn ensure_loaded_returns_draining_error() {
        let cfg = test_config();
        let resident_set = crate::engine::scheduling::ResidentSet::new(0);
        let backend = LlamaBackend::new(
            &cfg,
            resident_set,
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        backend.draining.store(true, Ordering::Release);
        let err = backend.ensure_loaded().await.unwrap_err();
        assert!(matches!(err, EngineError::BackendDraining));
    }

    #[tokio::test]
    async fn ensure_loaded_returns_poisoned_error() {
        let cfg = test_config();
        let resident_set = crate::engine::scheduling::ResidentSet::new(0);
        let backend = LlamaBackend::new(
            &cfg,
            resident_set,
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        backend.poisoned.store(true, Ordering::Release);
        let err = backend.ensure_loaded().await.unwrap_err();
        assert!(matches!(err, EngineError::BackendPoisoned));
    }

    #[tokio::test]
    async fn backend_state_starts_unloaded() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let state = backend.load_state.lock().await;
        assert!(matches!(&*state, LoadState::Unloaded));
    }

    #[tokio::test]
    async fn backend_touch_last_used_updates_timestamp() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let before = backend.last_used_ms.load(std::sync::atomic::Ordering::Acquire);
        std::thread::sleep(std::time::Duration::from_millis(2));
        backend.touch_last_used();
        let after = backend.last_used_ms.load(std::sync::atomic::Ordering::Acquire);
        assert!(after > before, "before={before} after={after}");
    }

    #[tokio::test]
    async fn in_flight_guard_increments_and_decrements() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        // Simulate a loaded state by directly transitioning — we're only
        // testing InFlightGuard bookkeeping, not the load path.
        {
            let (dummy_tx, _rx) = tokio::sync::mpsc::channel(1);
            let mut guard = backend.load_state.lock().await;
            *guard = LoadState::Loaded(LoadedState {
                cmd_tx: dummy_tx,
                worker_handle: None,
            });
        }

        assert_eq!(backend.in_flight.load(Ordering::Acquire), 0);
        {
            let _g = backend.ensure_loaded_for_request().await.unwrap();
            assert_eq!(backend.in_flight.load(Ordering::Acquire), 1);
        }
        assert_eq!(backend.in_flight.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn ensure_loaded_for_request_touches_last_used() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::ResidentSet::new(0),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        {
            let (dummy_tx, _rx) = tokio::sync::mpsc::channel(1);
            let mut guard = backend.load_state.lock().await;
            *guard = LoadState::Loaded(LoadedState {
                cmd_tx: dummy_tx,
                worker_handle: None,
            });
        }
        let before = backend.last_used_ms.load(Ordering::Acquire);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _g = backend.ensure_loaded_for_request().await.unwrap();
        let after = backend.last_used_ms.load(Ordering::Acquire);
        assert!(after > before);
    }
}
