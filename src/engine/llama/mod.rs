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
use crate::engine::scheduling::{Placement, ResolvedPlacement, Scheduler};
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

    scheduler: Arc<Scheduler>,
    estimated_vram_mb: u64,

    pub(crate) load_state: Mutex<LoadState>,
    /// Unix millis of most recent request completion or load success.
    pub(crate) last_used_ms: Arc<AtomicU64>,
    /// Count of requests currently executing + loading sentinel.
    pub(crate) in_flight: Arc<AtomicU32>,
    /// Process-wide mutex serializing load + evict sequences.
    load_coordinator: Arc<Mutex<()>>,
    /// Late-bound (set after registry construction).
    evictor: OnceCell<Weak<dyn Evictor>>,

    /// Placement state: `Auto` until first successful load, then
    /// `Resolved(Placement)` for the model's lifetime.
    pub(crate) placement: Mutex<ResolvedPlacement>,
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
        scheduler: Arc<Scheduler>,
        estimated_vram_mb: u64,
        load_coordinator: Arc<Mutex<()>>,
    ) -> Result<Self, EngineError> {
        if config.path.is_none() {
            return Err(EngineError::ModelLoadFailed(format!(
                "local backend '{}' has no path (config validation should have caught this)",
                config.id
            )));
        }
        let initial_placement = ResolvedPlacement::from_gpus(&config.gpus);
        Ok(Self {
            config: config.clone(),
            poisoned: Arc::new(AtomicBool::new(false)),
            draining: Arc::new(AtomicBool::new(false)),
            scheduler,
            estimated_vram_mb,
            load_state: Mutex::new(LoadState::Unloaded),
            last_used_ms: Arc::new(AtomicU64::new(0)),
            in_flight: Arc::new(AtomicU32::new(0)),
            load_coordinator,
            evictor: OnceCell::new(),
            placement: Mutex::new(initial_placement),
        })
    }

    /// Resolve the current placement. If Auto, pick the device with the
    /// most free budget via the scheduler; does NOT commit. Call
    /// `commit_placement` after a successful reservation to lock in the
    /// Auto-chosen device.
    pub(crate) async fn resolve_placement_or_fail(&self) -> Result<Placement, EngineError> {
        let guard = self.placement.lock().await;
        match &*guard {
            ResolvedPlacement::Resolved(p) => Ok(p.clone()),
            ResolvedPlacement::Auto => {
                let Some(gpu_id) = self.scheduler.pick_most_free_device() else {
                    return Err(EngineError::ModelUnavailable(
                        "auto-placement: no devices configured".into(),
                    ));
                };
                Ok(Placement::SingleDevice(gpu_id))
            }
        }
    }

    /// Lock in the chosen placement if the model was previously Auto.
    /// Idempotent.
    pub(crate) async fn commit_placement(&self, resolved: Placement) {
        let mut guard = self.placement.lock().await;
        if matches!(&*guard, ResolvedPlacement::Auto) {
            *guard = ResolvedPlacement::Resolved(resolved);
        }
    }

    pub(crate) fn touch_last_used(&self) {
        self.last_used_ms.store(now_unix_ms(), Ordering::Release);
    }

    /// Spawn the worker OS thread and drive it through Load. Returns the
    /// populated `LoadedState` on success. Task 14 adds the eviction loop
    /// around this call.
    async fn spawn_worker_and_send_load(
        &self,
        placement: &crate::engine::scheduling::Placement,
    ) -> Result<LoadedState, EngineError> {
        let (main_gpu, devices, split_mode) = placement.to_llama_args();

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
            .send(WorkerCommand::Load {
                main_gpu,
                devices,
                split_mode,
                ack: ack_tx,
            })
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

        if let Err(e) = self.try_reserve_with_eviction().await {
            self.fail_loading(&notify).await;
            metrics::counter!(
                "flarion_model_loads_total",
                "model" => self.config.id.clone(),
                "result" => "over_budget",
            )
            .increment(1);
            return Err(e);
        }

        // Reservation succeeded — commit_placement ran inside
        // try_reserve_with_eviction for Auto models. Read it out now.
        let resolved: Placement = {
            let guard = self.placement.lock().await;
            match &*guard {
                ResolvedPlacement::Resolved(p) => p.clone(),
                ResolvedPlacement::Auto => {
                    // Should not happen — try_reserve_with_eviction commits
                    // before returning Ok. Defensive bail.
                    self.fail_loading(&notify).await;
                    return Err(EngineError::InferenceFailed(
                        "placement remained Auto after successful reservation".into(),
                    ));
                }
            }
        };

        match self.spawn_worker_and_send_load(&resolved).await {
            Ok(loaded) => {
                {
                    let mut guard = self.load_state.lock().await;
                    *guard = LoadState::Loaded(loaded);
                }
                notify.notify_waiters();
                self.touch_last_used();
                // Emit per-device reserved gauges for every device this
                // placement touches.
                for (gpu_id, cost) in resolved.per_device_cost(self.estimated_vram_mb) {
                    crate::metrics::set_vram_reserved_on_gpu(&self.config.id, gpu_id, cost);
                }
                metrics::counter!(
                    "flarion_model_loads_total",
                    "model" => self.config.id.clone(),
                    "result" => "success",
                )
                .increment(1);
                Ok(())
            }
            Err(e) => {
                self.scheduler.release(&self.config.id, resolved.gpus());
                for (gpu_id, _) in resolved.per_device_cost(self.estimated_vram_mb) {
                    crate::metrics::set_vram_reserved_on_gpu(&self.config.id, gpu_id, 0);
                }
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

    /// Reserve `estimated_vram_mb` for `self.config.id`, evicting LRU
    /// victims as necessary. Returns Ok with the reservation held; caller
    /// must either `spawn_worker_and_send_load` or `release` on failure.
    pub(crate) async fn try_reserve_with_eviction(&self) -> Result<(), EngineError> {
        use crate::engine::scheduling::{ReservationContext, SchedulerError};
        loop {
            // Resolve placement on each iteration — Auto may migrate across
            // iterations based on live free-budget numbers.
            let resolved = self.resolve_placement_or_fail().await?;
            let per_device_costs = resolved.per_device_cost(self.estimated_vram_mb);

            let ctx = ReservationContext {
                model_id: &self.config.id,
                pinned: self.config.pin,
                last_used_ms: self.last_used_ms.clone(),
                in_flight: self.in_flight.clone(),
            };

            match self.scheduler.try_reserve_split(&per_device_costs, ctx) {
                Ok(()) => {
                    self.commit_placement(resolved).await;
                    return Ok(());
                }
                Err(SchedulerError::OverBudget { gpu_id, .. }) => {
                    let needed_mb = per_device_costs
                        .iter()
                        .find(|(g, _)| *g == gpu_id)
                        .map(|(_, c)| *c)
                        .expect("gpu_id from error must be in per_device_costs");

                    let Some(set) = self.scheduler.set(gpu_id) else {
                        return Err(EngineError::InferenceFailed(format!(
                            "placement targets unknown gpu {gpu_id}"
                        )));
                    };
                    let Some(victims) = set.pick_eviction_candidates(needed_mb) else {
                        return Err(EngineError::ModelUnavailable(format!(
                            "VRAM budget exceeded on gpu {gpu_id}: no eviction candidates available"
                        )));
                    };

                    let Some(evictor) = self.evictor.get().and_then(|w| w.upgrade()) else {
                        return Err(EngineError::ModelUnavailable(
                            "no evictor bound; cannot free VRAM".into(),
                        ));
                    };

                    let mut any_evicted = false;
                    for v in victims {
                        match evictor.unload(&v).await {
                            Ok(()) => {
                                metrics::counter!(
                                    "flarion_model_evictions_total",
                                    "model" => v.clone(),
                                    "reason" => "lru",
                                    "gpu" => gpu_id.to_string(),
                                )
                                .increment(1);
                                tracing::info!(
                                    victim_model_id = %v,
                                    gpu_id = gpu_id,
                                    "evicted model to free VRAM"
                                );
                                any_evicted = true;
                            }
                            Err(EngineError::BackendBusy) => continue,
                            Err(e) => return Err(e),
                        }
                    }
                    if !any_evicted {
                        return Err(EngineError::ModelUnavailable(format!(
                            "all eviction candidates busy on gpu {gpu_id}; retry shortly"
                        )));
                    }
                    // loop back — retry try_reserve_split
                }
                Err(SchedulerError::UnknownGpu { gpu_id }) => {
                    return Err(EngineError::InferenceFailed(format!(
                        "placement targets unknown gpu {gpu_id}"
                    )));
                }
                Err(SchedulerError::Poisoned { gpu_id }) => {
                    return Err(EngineError::InferenceFailed(format!(
                        "resident set poisoned for gpu {gpu_id}"
                    )));
                }
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

    async fn bind_evictor(&self, evictor: Weak<dyn Evictor>) {
        let _ = self.evictor.set(evictor);
    }

    async fn unload(&self) -> Result<(), EngineError> {
        let mut guard = self.load_state.lock().await;
        if self.in_flight.load(Ordering::Acquire) > 0 {
            return Err(EngineError::BackendBusy);
        }

        let loaded = match &mut *guard {
            LoadState::Loaded(_) => {
                if let LoadState::Loaded(l) = std::mem::replace(&mut *guard, LoadState::Unloaded) {
                    l
                } else {
                    unreachable!()
                }
            }
            LoadState::Unloaded => return Ok(()),
            LoadState::Loading(_) => return Err(EngineError::BackendBusy),
        };
        drop(guard);

        // Shutdown sequence: send, await ack with timeout, join worker.
        let (ack_tx, ack_rx) = oneshot::channel();
        let send_ok = loaded
            .cmd_tx
            .send(WorkerCommand::Shutdown { ack: ack_tx })
            .await
            .is_ok();
        drop(loaded.cmd_tx);

        if send_ok {
            let _ = tokio::time::timeout(Duration::from_secs(30), ack_rx).await;
        }

        if let Some(handle) = loaded.worker_handle {
            let join_res = tokio::task::spawn_blocking(move || handle.join()).await;
            if join_res.is_err() {
                tracing::warn!(
                    model_id = %self.config.id,
                    "worker thread panicked during unload"
                );
            }
        }

        // Release per-device reservations based on current placement.
        let placement_guard = self.placement.lock().await;
        let gpu_ids: Vec<u32> = match &*placement_guard {
            ResolvedPlacement::Auto => Vec::new(),
            ResolvedPlacement::Resolved(p) => p.gpus().to_vec(),
        };
        drop(placement_guard);
        self.scheduler.release(&self.config.id, &gpu_ids);
        for gpu_id in &gpu_ids {
            crate::metrics::set_vram_reserved_on_gpu(&self.config.id, *gpu_id, 0);
        }

        // Auto-placed models return to Auto so next load re-best-fits.
        if self.config.gpus.is_empty() {
            let mut placement_guard = self.placement.lock().await;
            *placement_guard = ResolvedPlacement::Auto;
        }

        metrics::counter!(
            "flarion_model_unloads_total",
            "model" => self.config.id.clone(),
            "result" => "success",
        )
        .increment(1);
        tracing::info!(model_id = %self.config.id, "model unloaded");
        Ok(())
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

        // Release per-device reservations based on current placement.
        let placement_guard = self.placement.lock().await;
        let gpu_ids: Vec<u32> = match &*placement_guard {
            ResolvedPlacement::Auto => Vec::new(),
            ResolvedPlacement::Resolved(p) => p.gpus().to_vec(),
        };
        drop(placement_guard);
        self.scheduler.release(&self.config.id, &gpu_ids);
        for gpu_id in &gpu_ids {
            crate::metrics::set_vram_reserved_on_gpu(&self.config.id, *gpu_id, 0);
        }
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
            gpus: vec![],
            repo: None,
            revision: None,
            dtype: None,
            hf_token_env: None,
            adapters: Vec::new(),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
        let sched = crate::engine::scheduling::Scheduler::new(vec![0]);
        let backend = LlamaBackend::new(
            &cfg,
            sched,
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let state = backend.load_state.lock().await;
        assert!(matches!(&*state, LoadState::Unloaded));
    }

    #[tokio::test]
    async fn ensure_loaded_returns_over_budget_when_resident_set_full() {
        use crate::engine::scheduling::{ReservationRequest, Scheduler};
        let cfg = test_config();
        let sched = Scheduler::new(vec![1000]);
        sched.set(0).unwrap()
            .try_reserve(ReservationRequest {
                model_id: "other",
                cost_mb: 1000,
                pinned: false,
                last_used_ms: Arc::new(AtomicU64::new(0)),
                in_flight: Arc::new(AtomicU32::new(0)),
            })
            .unwrap();
        let backend = LlamaBackend::new(
            &cfg,
            sched,
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
        let sched = crate::engine::scheduling::Scheduler::new(vec![0]);
        let backend = LlamaBackend::new(
            &cfg,
            sched,
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
        let sched = crate::engine::scheduling::Scheduler::new(vec![0]);
        let backend = LlamaBackend::new(
            &cfg,
            sched,
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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
            crate::engine::scheduling::Scheduler::new(vec![0]),
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

    #[tokio::test]
    async fn unload_when_unloaded_is_noop() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::Scheduler::new(vec![0]),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        backend.unload().await.unwrap();
        let state = backend.load_state.lock().await;
        assert!(matches!(&*state, LoadState::Unloaded));
    }

    #[tokio::test]
    async fn unload_when_busy_returns_backend_busy() {
        let backend = LlamaBackend::new(
            &test_config(),
            crate::engine::scheduling::Scheduler::new(vec![0]),
            0,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        // Fake Loaded + in_flight=1.
        {
            let (dummy_tx, _rx) = tokio::sync::mpsc::channel(1);
            let mut guard = backend.load_state.lock().await;
            *guard = LoadState::Loaded(LoadedState {
                cmd_tx: dummy_tx,
                worker_handle: None,
            });
        }
        backend.in_flight.store(1, Ordering::Release);
        let err = backend.unload().await.unwrap_err();
        assert!(matches!(err, EngineError::BackendBusy), "got {err:?}");
        // State should still be Loaded.
        let state = backend.load_state.lock().await;
        assert!(matches!(&*state, LoadState::Loaded(_)));
    }

    #[tokio::test]
    async fn eviction_loop_unloads_lru_victim_and_releases_budget() {
        use crate::engine::backend::Evictor;
        use crate::engine::scheduling::{ReservationRequest, Scheduler};

        // A stub evictor that records calls and releases the reservation,
        // mirroring what the real `LlamaBackend::unload` does (it is the sole
        // authority for releasing on the scheduler after a successful unload).
        struct StubEvictor {
            calls: Arc<std::sync::Mutex<Vec<String>>>,
            set0: Arc<crate::engine::scheduling::ResidentSet>,
        }
        #[async_trait]
        impl Evictor for StubEvictor {
            async fn unload(&self, id: &str) -> Result<(), EngineError> {
                self.calls.lock().unwrap().push(id.to_string());
                self.set0.release(id);
                crate::metrics::set_vram_reserved_on_gpu(id, 0, 0);
                Ok(())
            }
        }

        let sched = Scheduler::new(vec![1000]);
        let set0 = sched.set(0).unwrap();
        set0.try_reserve(ReservationRequest {
            model_id: "old",
            cost_mb: 800,
            pinned: false,
            last_used_ms: Arc::new(AtomicU64::new(100)),
            in_flight: Arc::new(AtomicU32::new(0)),
        })
        .unwrap();

        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let stub: Arc<dyn Evictor> = Arc::new(StubEvictor {
            calls: calls.clone(),
            set0: set0.clone(),
        });
        let weak = Arc::downgrade(&stub);

        let mut cfg = test_config();
        cfg.lazy = true;
        let backend = LlamaBackend::new(
            &cfg,
            sched,
            600, // needs 600 MB, only 200 MB free → must evict "old"
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        backend.bind_evictor(weak).await;

        // Drive the reservation+eviction helper directly. We do NOT call
        // ensure_loaded (which would proceed to spawn_worker_and_send_load
        // and fail on the missing GGUF). This isolates the eviction logic.
        let result = backend.try_reserve_with_eviction().await;
        assert!(result.is_ok(), "reservation failed: {result:?}");
        assert_eq!(calls.lock().unwrap().as_slice(), &["old".to_string()]);
        assert_eq!(set0.total_reserved_mb(), 600);
    }

    #[tokio::test]
    async fn two_parallel_lazy_loads_serialize_via_coordinator() {
        // This test validates only the coordinator mutex — not the worker
        // spawn, which requires a real GGUF. We drive try_reserve_with_eviction
        // on two backends sharing the same resident_set + load_coordinator and
        // assert that the total reserved never exceeds budget.
        use crate::engine::backend::Evictor;

        // Stub evictor that always refuses (no-op unload returns BackendBusy) —
        // tests that the FIRST reserver wins without interference.
        struct RefusingEvictor;
        #[async_trait]
        impl Evictor for RefusingEvictor {
            async fn unload(&self, _id: &str) -> Result<(), EngineError> {
                Err(EngineError::BackendBusy)
            }
        }

        let sched = crate::engine::scheduling::Scheduler::new(vec![1000]);
        let set0 = sched.set(0).unwrap();
        let coord: Arc<tokio::sync::Mutex<()>> = Arc::new(tokio::sync::Mutex::new(()));
        let stub: Arc<dyn Evictor> = Arc::new(RefusingEvictor);
        let weak = Arc::downgrade(&stub);

        let mut cfg_a = test_config();
        cfg_a.id = "a".into();
        cfg_a.lazy = true;
        let a = Arc::new(LlamaBackend::new(&cfg_a, sched.clone(), 600, coord.clone()).unwrap());
        a.bind_evictor(weak.clone()).await;

        let mut cfg_b = test_config();
        cfg_b.id = "b".into();
        cfg_b.lazy = true;
        let b = Arc::new(LlamaBackend::new(&cfg_b, sched.clone(), 600, coord.clone()).unwrap());
        b.bind_evictor(weak.clone()).await;

        // Drive both reservations concurrently with the SAME load_coordinator
        // held manually around each call so serialization is observable in
        // this test (production serializes via load_as_leader, not directly).
        let a2 = a.clone();
        let b2 = b.clone();
        let coord_a = coord.clone();
        let coord_b = coord.clone();
        let fa = tokio::spawn(async move {
            let _g = coord_a.lock().await;
            a2.try_reserve_with_eviction().await
        });
        let fb = tokio::spawn(async move {
            let _g = coord_b.lock().await;
            b2.try_reserve_with_eviction().await
        });

        let (ra, rb) = tokio::join!(fa, fb);
        let ra = ra.unwrap();
        let rb = rb.unwrap();

        // Exactly one succeeded (the first to grab coord); the other failed
        // because budget was full and RefusingEvictor refuses every unload.
        let succ = ra.is_ok() as u32 + rb.is_ok() as u32;
        assert_eq!(succ, 1, "expected exactly one success, got {ra:?} / {rb:?}");
        assert_eq!(set0.total_reserved_mb(), 600);
    }

    #[tokio::test]
    async fn backend_constructs_with_scheduler_and_auto_placement() {
        use crate::engine::scheduling::{ResolvedPlacement, Scheduler};
        let sched = Scheduler::new(vec![1000, 1000]);
        let cfg = test_config(); // gpus = []
        let backend = LlamaBackend::new(
            &cfg,
            sched,
            500,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let p = backend.placement.lock().await;
        assert!(matches!(&*p, ResolvedPlacement::Auto));
    }

    #[tokio::test]
    async fn backend_constructs_with_single_device_placement() {
        use crate::engine::scheduling::{Placement, ResolvedPlacement, Scheduler};
        let sched = Scheduler::new(vec![1000, 1000]);
        let mut cfg = test_config();
        cfg.gpus = vec![1];
        let backend = LlamaBackend::new(
            &cfg,
            sched,
            500,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let p = backend.placement.lock().await;
        match &*p {
            ResolvedPlacement::Resolved(Placement::SingleDevice(n)) => assert_eq!(*n, 1),
            other => panic!("got {other:?}"),
        }
    }

    #[tokio::test]
    async fn backend_resolves_auto_placement_to_most_free_device() {
        use crate::engine::scheduling::{Placement, Scheduler};
        let sched = Scheduler::new(vec![500, 1000]); // gpu 1 has more free
        let cfg = test_config();
        let backend = LlamaBackend::new(
            &cfg,
            sched.clone(),
            200,
            Arc::new(tokio::sync::Mutex::new(())),
        )
        .unwrap();
        let p = backend.resolve_placement_or_fail().await.unwrap();
        assert_eq!(p, Placement::SingleDevice(1));
    }

    #[tokio::test]
    async fn multi_device_reserve_with_eviction_reserves_on_all_gpus_for_split_model() {
        use crate::engine::backend::Evictor;
        use crate::engine::scheduling::Scheduler;

        struct NoopEvictor;
        #[async_trait]
        impl Evictor for NoopEvictor {
            async fn unload(&self, _id: &str) -> Result<(), EngineError> {
                Ok(())
            }
        }

        let sched = Scheduler::new(vec![10000, 10000]);
        let coord = Arc::new(tokio::sync::Mutex::new(()));
        let evictor: Arc<dyn Evictor> = Arc::new(NoopEvictor);
        let weak = Arc::downgrade(&evictor);

        let mut cfg = test_config();
        cfg.id = "split".into();
        cfg.gpus = vec![0, 1];
        cfg.lazy = true;
        let backend = LlamaBackend::new(&cfg, sched.clone(), 10000, coord).unwrap();
        backend.bind_evictor(weak).await;

        backend.try_reserve_with_eviction().await.unwrap();

        // 10000 MB split uniformly across 2 devices = 5000 each.
        assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 5000);
        assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 5000);
    }

    #[tokio::test]
    async fn multi_device_reserve_with_eviction_commits_auto_placement() {
        use crate::engine::backend::Evictor;
        use crate::engine::scheduling::{Placement, ResolvedPlacement, Scheduler};

        struct NoopEvictor;
        #[async_trait]
        impl Evictor for NoopEvictor {
            async fn unload(&self, _id: &str) -> Result<(), EngineError> {
                Ok(())
            }
        }

        // Device 0 has less free (800); device 1 has more (2000).
        let sched = Scheduler::new(vec![800, 2000]);
        let coord = Arc::new(tokio::sync::Mutex::new(()));
        let evictor: Arc<dyn Evictor> = Arc::new(NoopEvictor);
        let weak = Arc::downgrade(&evictor);

        let mut cfg = test_config();
        cfg.id = "auto".into();
        cfg.gpus = vec![]; // Auto
        cfg.lazy = true;
        let backend = LlamaBackend::new(&cfg, sched.clone(), 500, coord).unwrap();
        backend.bind_evictor(weak).await;

        backend.try_reserve_with_eviction().await.unwrap();

        // Best-fit picked gpu 1 (more free).
        let placement = backend.placement.lock().await;
        match &*placement {
            ResolvedPlacement::Resolved(Placement::SingleDevice(n)) => assert_eq!(*n, 1),
            other => panic!("expected SingleDevice(1), got {other:?}"),
        }
        assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 500);
        assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 0);
    }

    #[tokio::test]
    async fn unload_releases_split_model_reservations_on_all_gpus() {
        use crate::engine::backend::Evictor;
        use crate::engine::scheduling::Scheduler;

        struct NoopEvictor;
        #[async_trait]
        impl Evictor for NoopEvictor {
            async fn unload(&self, _id: &str) -> Result<(), EngineError> {
                Ok(())
            }
        }

        let sched = Scheduler::new(vec![10000, 10000]);
        let coord = Arc::new(tokio::sync::Mutex::new(()));
        let evictor: Arc<dyn Evictor> = Arc::new(NoopEvictor);

        let mut cfg = test_config();
        cfg.id = "split".into();
        cfg.gpus = vec![0, 1];
        cfg.lazy = true;
        let backend = LlamaBackend::new(&cfg, sched.clone(), 10000, coord).unwrap();
        backend.bind_evictor(Arc::downgrade(&evictor)).await;

        backend.try_reserve_with_eviction().await.unwrap();

        // Fake Loaded state (cannot spawn llama-cpp without a GGUF).
        {
            let (dummy_tx, _rx) = tokio::sync::mpsc::channel(1);
            let mut guard = backend.load_state.lock().await;
            *guard = LoadState::Loaded(LoadedState {
                cmd_tx: dummy_tx,
                worker_handle: None,
            });
        }

        backend.unload().await.unwrap();

        // Both devices released.
        assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 0);
        assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 0);
    }

    #[tokio::test]
    async fn unload_of_auto_placed_model_resets_placement_to_auto() {
        use crate::engine::backend::Evictor;
        use crate::engine::scheduling::{ResolvedPlacement, Scheduler};

        struct NoopEvictor;
        #[async_trait]
        impl Evictor for NoopEvictor {
            async fn unload(&self, _id: &str) -> Result<(), EngineError> {
                Ok(())
            }
        }

        let sched = Scheduler::new(vec![10000, 10000]);
        let coord = Arc::new(tokio::sync::Mutex::new(()));
        let evictor: Arc<dyn Evictor> = Arc::new(NoopEvictor);

        let mut cfg = test_config();
        cfg.id = "auto".into();
        cfg.gpus = vec![]; // Auto
        cfg.lazy = true;
        let backend = LlamaBackend::new(&cfg, sched.clone(), 1000, coord).unwrap();
        backend.bind_evictor(Arc::downgrade(&evictor)).await;

        backend.try_reserve_with_eviction().await.unwrap();

        // Placement should be committed after reserve.
        {
            let p = backend.placement.lock().await;
            assert!(matches!(&*p, ResolvedPlacement::Resolved(_)));
        }

        // Fake Loaded state.
        {
            let (dummy_tx, _rx) = tokio::sync::mpsc::channel(1);
            let mut guard = backend.load_state.lock().await;
            *guard = LoadState::Loaded(LoadedState {
                cmd_tx: dummy_tx,
                worker_handle: None,
            });
        }

        backend.unload().await.unwrap();

        // Auto models reset after unload so next load re-best-fits.
        let p = backend.placement.lock().await;
        assert!(matches!(&*p, ResolvedPlacement::Auto));
    }

    #[tokio::test]
    async fn unload_of_explicit_model_does_not_reset_placement() {
        use crate::engine::backend::Evictor;
        use crate::engine::scheduling::{Placement, ResolvedPlacement, Scheduler};

        struct NoopEvictor;
        #[async_trait]
        impl Evictor for NoopEvictor {
            async fn unload(&self, _id: &str) -> Result<(), EngineError> {
                Ok(())
            }
        }

        let sched = Scheduler::new(vec![10000, 10000]);
        let coord = Arc::new(tokio::sync::Mutex::new(()));
        let evictor: Arc<dyn Evictor> = Arc::new(NoopEvictor);

        let mut cfg = test_config();
        cfg.id = "pinned_gpu_1".into();
        cfg.gpus = vec![1];
        cfg.lazy = true;
        let backend = LlamaBackend::new(&cfg, sched.clone(), 1000, coord).unwrap();
        backend.bind_evictor(Arc::downgrade(&evictor)).await;

        backend.try_reserve_with_eviction().await.unwrap();
        {
            let (dummy_tx, _rx) = tokio::sync::mpsc::channel(1);
            let mut guard = backend.load_state.lock().await;
            *guard = LoadState::Loaded(LoadedState {
                cmd_tx: dummy_tx,
                worker_handle: None,
            });
        }
        backend.unload().await.unwrap();

        // Placement remains Resolved(SingleDevice(1)) across unload.
        let p = backend.placement.lock().await;
        match &*p {
            ResolvedPlacement::Resolved(Placement::SingleDevice(n)) => assert_eq!(*n, 1),
            other => panic!("got {other:?}"),
        }
    }
}
