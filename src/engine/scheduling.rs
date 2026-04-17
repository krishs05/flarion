//! VRAM budget tracking and estimation for local model scheduling.
//!
//! `ResidentSet` is shared state (behind an `Arc`) that tracks which local
//! models are currently loaded and how much VRAM they reserve. `try_reserve`
//! is called inside `LlamaBackend::try_load_inner` atomically with the
//! worker-spawn + llama.cpp load sequence.
//!
//! `estimate_vram_mb` produces a per-model footprint — either a user-provided
//! override or `file_size * 1.2` when no override is given.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, Mutex};

pub use llama_cpp_2::model::params::LlamaSplitMode;

/// Concrete placement for a loaded model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    SingleDevice(u32),
    TensorSplit {
        gpus: Vec<u32>, // device ids, length >= 2
    },
}

/// Placement state on a `LlamaBackend`. `Auto` defers to the scheduler's
/// best-fit selection at first load.
#[derive(Debug, Clone)]
pub enum ResolvedPlacement {
    Auto,
    Resolved(Placement),
}

impl ResolvedPlacement {
    /// Parse the raw `gpus: Vec<u32>` config field into a resolved state.
    /// - `[]` → Auto
    /// - `[N]` → Resolved(SingleDevice(N))
    /// - `[N, M, ...]` (len ≥ 2) → Resolved(TensorSplit { gpus })
    pub fn from_gpus(gpus: &[u32]) -> Self {
        match gpus.len() {
            0 => ResolvedPlacement::Auto,
            1 => ResolvedPlacement::Resolved(Placement::SingleDevice(gpus[0])),
            _ => ResolvedPlacement::Resolved(Placement::TensorSplit {
                gpus: gpus.to_vec(),
            }),
        }
    }
}

impl Placement {
    /// Flat list of gpu_ids this placement touches.
    pub fn gpus(&self) -> &[u32] {
        match self {
            Placement::SingleDevice(n) => std::slice::from_ref(n),
            Placement::TensorSplit { gpus } => gpus,
        }
    }

    /// Per-device cost contribution for a given total model estimate.
    /// Uses a uniform split heuristic with remainder on the first device.
    pub fn per_device_cost(&self, total_mb: u64) -> Vec<(u32, u64)> {
        match self {
            Placement::SingleDevice(n) => vec![(*n, total_mb)],
            Placement::TensorSplit { gpus } => {
                let n = gpus.len() as u64;
                let base = total_mb / n;
                let remainder = total_mb - base * n;
                gpus.iter()
                    .enumerate()
                    .map(|(i, &g)| (g, if i == 0 { base + remainder } else { base }))
                    .collect()
            }
        }
    }

    /// Corresponding llama-cpp-2 load arguments: (main_gpu, devices, split_mode).
    pub fn to_llama_args(&self) -> (u32, Vec<usize>, LlamaSplitMode) {
        match self {
            Placement::SingleDevice(n) => (*n, vec![*n as usize], LlamaSplitMode::None),
            Placement::TensorSplit { gpus } => (
                gpus[0],
                gpus.iter().map(|&g| g as usize).collect(),
                LlamaSplitMode::Layer,
            ),
        }
    }
}

#[derive(Debug)]
struct ResidentEntry {
    cost_mb: u64,
    last_used_ms: Arc<AtomicU64>,
    in_flight: Arc<AtomicU32>,
    pinned: bool,
}

/// What a caller passes to `ResidentSet::try_reserve`. The `last_used_ms` /
/// `in_flight` handles are shared with the owning `LlamaBackend` so the
/// eviction driver observes live values without a lock dance.
pub struct ReservationRequest<'a> {
    pub model_id: &'a str,
    pub cost_mb: u64,
    pub pinned: bool,
    pub last_used_ms: Arc<AtomicU64>,
    pub in_flight: Arc<AtomicU32>,
}

pub struct ResidentSet {
    inner: Mutex<Inner>,
    budget_mb: u64,
}

struct Inner {
    loaded: HashMap<String, ResidentEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResidentError {
    #[error(
        "VRAM over budget for '{model_id}': requested {requested_mb}MB, current {current_mb}MB, budget {budget_mb}MB"
    )]
    OverBudget {
        model_id: String,
        requested_mb: u64,
        current_mb: u64,
        budget_mb: u64,
    },
    #[error("resident set lock poisoned")]
    Poisoned,
}

impl ResidentSet {
    pub fn new(budget_mb: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                loaded: HashMap::new(),
            }),
            budget_mb,
        })
    }

    pub fn budget_mb(&self) -> u64 {
        self.budget_mb
    }

    pub fn total_reserved_mb(&self) -> u64 {
        self.inner
            .lock()
            .map(|inner| inner.loaded.values().map(|e| e.cost_mb).sum())
            .unwrap_or(0)
    }

    /// Reserve `req.cost_mb` for `req.model_id`, recording metadata for
    /// eviction decisions. Idempotent for the same id — subsequent reserves
    /// refresh the metadata (pin, last_used, in_flight handles) without
    /// changing `cost_mb` or budget accounting. Zero-budget sets always
    /// succeed.
    pub fn try_reserve(&self, req: ReservationRequest<'_>) -> Result<(), ResidentError> {
        if self.budget_mb == 0 {
            return Ok(());
        }

        let mut inner = self.inner.lock().map_err(|_| ResidentError::Poisoned)?;

        if let Some(existing) = inner.loaded.get_mut(req.model_id) {
            existing.pinned = req.pinned;
            existing.last_used_ms = req.last_used_ms;
            existing.in_flight = req.in_flight;
            return Ok(());
        }

        let current: u64 = inner.loaded.values().map(|e| e.cost_mb).sum();
        if current.saturating_add(req.cost_mb) > self.budget_mb {
            return Err(ResidentError::OverBudget {
                model_id: req.model_id.to_string(),
                requested_mb: req.cost_mb,
                current_mb: current,
                budget_mb: self.budget_mb,
            });
        }

        inner.loaded.insert(
            req.model_id.to_string(),
            ResidentEntry {
                cost_mb: req.cost_mb,
                last_used_ms: req.last_used_ms,
                in_flight: req.in_flight,
                pinned: req.pinned,
            },
        );
        Ok(())
    }

    /// Pick a list of currently-loaded model ids to evict in LRU order so
    /// that releasing their combined `cost_mb` would free at least
    /// `needed_mb`. Skips pinned models and models with `in_flight > 0`.
    /// Returns `None` when no feasible combination of eligible candidates
    /// can free enough.
    pub fn pick_eviction_candidates(&self, needed_mb: u64) -> Option<Vec<String>> {
        if self.budget_mb == 0 {
            return None;
        }
        let inner = self.inner.lock().ok()?;

        // Snapshot eligible entries.
        use std::sync::atomic::Ordering;
        let mut candidates: Vec<(String, u64, u64)> = inner
            .loaded
            .iter()
            .filter_map(|(id, entry)| {
                if entry.pinned {
                    return None;
                }
                if entry.in_flight.load(Ordering::Acquire) > 0 {
                    return None;
                }
                Some((id.clone(), entry.cost_mb, entry.last_used_ms.load(Ordering::Acquire)))
            })
            .collect();

        // Sort ascending by last_used_ms (oldest first); stable tie-break.
        candidates.sort_by_key(|(_, _, last_used)| *last_used);

        // Accumulate until sum >= needed_mb.
        let mut chosen = Vec::new();
        let mut sum: u64 = 0;
        for (id, cost, _) in candidates {
            chosen.push(id);
            sum = sum.saturating_add(cost);
            if sum >= needed_mb {
                return Some(chosen);
            }
        }
        None
    }

    /// Release the reservation for `model_id`. No-op if not reserved.
    pub fn release(&self, model_id: &str) {
        if self.budget_mb == 0 {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            inner.loaded.remove(model_id);
        }
    }
}

/// Shared context for reservations — the handles and metadata every
/// per-device entry needs. One `ReservationContext` drives N per-device
/// `ReservationRequest`s during a multi-device reservation.
pub struct ReservationContext<'a> {
    pub model_id: &'a str,
    pub pinned: bool,
    pub last_used_ms: Arc<AtomicU64>,
    pub in_flight: Arc<AtomicU32>,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("VRAM over budget on gpu {gpu_id}: {inner}")]
    OverBudget {
        gpu_id: u32,
        #[source]
        inner: ResidentError,
    },
    #[error("scheduler target gpu {gpu_id} is not configured")]
    UnknownGpu { gpu_id: u32 },
    #[error("resident set for gpu {gpu_id} poisoned")]
    Poisoned { gpu_id: u32 },
}

/// Multi-device VRAM scheduler. Owns one `ResidentSet` per GPU.
pub struct Scheduler {
    sets: Vec<Arc<ResidentSet>>,
}

impl Scheduler {
    /// Build a scheduler with one `ResidentSet` per entry in `budgets`.
    /// `budgets[i]` is the budget (MB) for gpu_id = i. Passing an empty
    /// `Vec` creates a scheduler with zero devices.
    pub fn new(budgets: Vec<u64>) -> Arc<Self> {
        let sets = budgets.into_iter().map(ResidentSet::new).collect();
        Arc::new(Self { sets })
    }

    pub fn device_count(&self) -> u32 {
        self.sets.len() as u32
    }

    pub fn set(&self, gpu_id: u32) -> Option<Arc<ResidentSet>> {
        self.sets.get(gpu_id as usize).cloned()
    }

    /// Returns the gpu_id with the most free budget. Used by
    /// auto-placement: callers don't filter by "fits now" — if the
    /// chosen device doesn't have enough room, the normal
    /// reserve+evict loop handles it. Returns `None` only when
    /// `device_count == 0`.
    pub fn pick_most_free_device(&self) -> Option<u32> {
        self.sets
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let free = s.budget_mb().saturating_sub(s.total_reserved_mb());
                (i as u32, free)
            })
            .max_by_key(|(_, free)| *free)
            .map(|(id, _)| id)
    }

    /// Atomic-ish multi-device reservation. Iterates `per_device_costs`
    /// in order; on any per-device failure, rolls back prior reservations
    /// made in this call and returns the underlying error.
    pub fn try_reserve_split(
        &self,
        per_device_costs: &[(u32, u64)],
        ctx: ReservationContext<'_>,
    ) -> Result<(), SchedulerError> {
        let mut reserved: Vec<u32> = Vec::with_capacity(per_device_costs.len());
        for &(gpu_id, cost_mb) in per_device_costs {
            let Some(set) = self.set(gpu_id) else {
                for &id in &reserved {
                    if let Some(s) = self.set(id) {
                        s.release(ctx.model_id);
                    }
                }
                return Err(SchedulerError::UnknownGpu { gpu_id });
            };
            let req = ReservationRequest {
                model_id: ctx.model_id,
                cost_mb,
                pinned: ctx.pinned,
                last_used_ms: ctx.last_used_ms.clone(),
                in_flight: ctx.in_flight.clone(),
            };
            match set.try_reserve(req) {
                Ok(()) => {
                    reserved.push(gpu_id);
                }
                Err(e) => {
                    for &id in &reserved {
                        if let Some(s) = self.set(id) {
                            s.release(ctx.model_id);
                        }
                    }
                    return Err(match e {
                        ResidentError::Poisoned => SchedulerError::Poisoned { gpu_id },
                        other => SchedulerError::OverBudget {
                            gpu_id,
                            inner: other,
                        },
                    });
                }
            }
        }
        Ok(())
    }

    /// Release the model's reservation on every gpu_id in `gpu_ids`.
    pub fn release(&self, model_id: &str, gpu_ids: &[u32]) {
        for &id in gpu_ids {
            if let Some(s) = self.set(id) {
                s.release(model_id);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EstimateError {
    #[error("failed to stat {path}: {source}")]
    StatFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Estimate a GGUF model's VRAM footprint in MB.
///
/// If `override_mb` is `Some`, return it verbatim without touching the
/// filesystem. Otherwise compute `file_size * 6 / 5` (i.e. 1.2×) and
/// convert to MB.
pub fn estimate_vram_mb(path: &Path, override_mb: Option<u64>) -> Result<u64, EstimateError> {
    if let Some(n) = override_mb {
        return Ok(n);
    }
    let metadata = std::fs::metadata(path).map_err(|source| EstimateError::StatFailed {
        path: path.to_path_buf(),
        source,
    })?;
    let size_bytes = metadata.len();
    let padded = size_bytes.saturating_mul(6) / 5;
    Ok(padded / (1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{Placement, ResolvedPlacement};
    use llama_cpp_2::model::params::LlamaSplitMode;

    #[test]
    fn placement_from_gpus_empty_produces_auto() {
        let rp = ResolvedPlacement::from_gpus(&[]);
        assert!(matches!(rp, ResolvedPlacement::Auto));
    }

    #[test]
    fn placement_from_gpus_single_produces_single_device() {
        let rp = ResolvedPlacement::from_gpus(&[3]);
        match rp {
            ResolvedPlacement::Resolved(Placement::SingleDevice(n)) => assert_eq!(n, 3),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn placement_from_gpus_multi_produces_tensor_split() {
        let rp = ResolvedPlacement::from_gpus(&[0, 1]);
        match rp {
            ResolvedPlacement::Resolved(Placement::TensorSplit { gpus }) => {
                assert_eq!(gpus, vec![0, 1]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn placement_per_device_cost_single_device() {
        let p = Placement::SingleDevice(2);
        assert_eq!(p.per_device_cost(1000), vec![(2, 1000)]);
    }

    #[test]
    fn placement_per_device_cost_split_uniform() {
        let p = Placement::TensorSplit { gpus: vec![0, 1] };
        assert_eq!(p.per_device_cost(10000), vec![(0, 5000), (1, 5000)]);
    }

    #[test]
    fn placement_per_device_cost_split_remainder_on_first() {
        // Uneven total: 1001 / 3 = 333 r 2. First gets 335, rest get 333.
        let p = Placement::TensorSplit { gpus: vec![0, 1, 2] };
        assert_eq!(p.per_device_cost(1001), vec![(0, 335), (1, 333), (2, 333)]);
    }

    #[test]
    fn placement_to_llama_args_single_device() {
        let p = Placement::SingleDevice(1);
        let (main_gpu, devices, mode) = p.to_llama_args();
        assert_eq!(main_gpu, 1);
        assert_eq!(devices, vec![1usize]);
        assert!(matches!(mode, LlamaSplitMode::None));
    }

    #[test]
    fn placement_to_llama_args_tensor_split() {
        let p = Placement::TensorSplit { gpus: vec![0, 1, 2] };
        let (main_gpu, devices, mode) = p.to_llama_args();
        assert_eq!(main_gpu, 0);
        assert_eq!(devices, vec![0usize, 1, 2]);
        assert!(matches!(mode, LlamaSplitMode::Layer));
    }

    #[test]
    fn placement_gpus_returns_all_target_devices() {
        assert_eq!(Placement::SingleDevice(3).gpus(), &[3][..]);
        assert_eq!(
            Placement::TensorSplit { gpus: vec![0, 1] }.gpus(),
            &[0, 1][..]
        );
    }

    fn mk_req(model_id: &str, cost_mb: u64) -> ReservationRequest<'_> {
        use std::sync::atomic::{AtomicU32, AtomicU64};
        ReservationRequest {
            model_id,
            cost_mb,
            pinned: false,
            last_used_ms: Arc::new(AtomicU64::new(0)),
            in_flight: Arc::new(AtomicU32::new(0)),
        }
    }

    #[test]
    fn reserve_within_budget_succeeds() {
        let set = ResidentSet::new(4000);
        assert!(set.try_reserve(mk_req("m", 1000)).is_ok());
        assert_eq!(set.total_reserved_mb(), 1000);
    }

    #[test]
    fn reserve_exceeding_budget_returns_err() {
        let set = ResidentSet::new(4000);
        let err = set.try_reserve(mk_req("m", 5000)).unwrap_err();
        match err {
            ResidentError::OverBudget {
                model_id,
                requested_mb,
                current_mb,
                budget_mb,
            } => {
                assert_eq!(model_id, "m");
                assert_eq!(requested_mb, 5000);
                assert_eq!(current_mb, 0);
                assert_eq!(budget_mb, 4000);
            }
            other => panic!("wrong error variant: {other:?}"),
        }
        assert_eq!(set.total_reserved_mb(), 0);
    }

    #[test]
    fn reserve_summed_across_models() {
        let set = ResidentSet::new(4000);
        assert!(set.try_reserve(mk_req("a", 1000)).is_ok());
        assert!(set.try_reserve(mk_req("b", 2000)).is_ok());
        assert!(set.try_reserve(mk_req("c", 2000)).is_err());
        assert_eq!(set.total_reserved_mb(), 3000);
    }

    #[test]
    fn reserve_is_idempotent_for_same_id() {
        let set = ResidentSet::new(4000);
        assert!(set.try_reserve(mk_req("m", 1000)).is_ok());
        assert!(set.try_reserve(mk_req("m", 999_999)).is_ok());
        assert_eq!(set.total_reserved_mb(), 1000);
    }

    #[test]
    fn release_removes_reservation() {
        let set = ResidentSet::new(4000);
        set.try_reserve(mk_req("m", 1000)).unwrap();
        set.release("m");
        assert_eq!(set.total_reserved_mb(), 0);
        assert!(set.try_reserve(mk_req("m", 2000)).is_ok());
        assert_eq!(set.total_reserved_mb(), 2000);
    }

    #[test]
    fn zero_budget_means_disabled() {
        let set = ResidentSet::new(0);
        assert!(set.try_reserve(mk_req("m", u64::MAX)).is_ok());
        assert_eq!(set.total_reserved_mb(), 0);
        set.release("m");
    }

    #[test]
    fn overbudget_error_carries_detail() {
        let set = ResidentSet::new(4000);
        set.try_reserve(mk_req("existing", 3000)).unwrap();
        let err = set.try_reserve(mk_req("new", 2000)).unwrap_err();
        match err {
            ResidentError::OverBudget {
                model_id,
                requested_mb,
                current_mb,
                budget_mb,
            } => {
                assert_eq!(model_id, "new");
                assert_eq!(requested_mb, 2000);
                assert_eq!(current_mb, 3000);
                assert_eq!(budget_mb, 4000);
            }
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    #[test]
    fn estimate_uses_override_when_set() {
        let nonexistent = std::path::PathBuf::from("/does/not/exist.gguf");
        let got = estimate_vram_mb(&nonexistent, Some(1500)).unwrap();
        assert_eq!(got, 1500);
    }

    #[test]
    fn estimate_uses_file_size_when_no_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.gguf");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(100 * 1024 * 1024).unwrap();
        drop(f);

        let got = estimate_vram_mb(&path, None).unwrap();
        assert!((119..=121).contains(&got), "got {got}");
    }

    #[test]
    fn estimate_missing_file_returns_err() {
        let nonexistent = std::path::PathBuf::from("/does/not/exist.gguf");
        let err = estimate_vram_mb(&nonexistent, None).unwrap_err();
        assert!(matches!(err, EstimateError::StatFailed { .. }));
    }

    #[test]
    fn reserve_request_carries_metadata() {
        let set = ResidentSet::new(4000);
        let last_used = Arc::new(AtomicU64::new(123));
        let in_flight = Arc::new(AtomicU32::new(0));
        set.try_reserve(ReservationRequest {
            model_id: "m",
            cost_mb: 1000,
            pinned: true,
            last_used_ms: last_used.clone(),
            in_flight: in_flight.clone(),
        })
        .unwrap();
        assert_eq!(set.total_reserved_mb(), 1000);
        // Proves pinned flag was actually stored: pick_eviction_candidates
        // must skip the only reserved entry.
        assert!(set.pick_eviction_candidates(500).is_none());
    }

    #[test]
    fn reserve_request_idempotent_for_same_id() {
        let set = ResidentSet::new(4000);
        let last_used = Arc::new(AtomicU64::new(0));
        let in_flight = Arc::new(AtomicU32::new(0));
        for _ in 0..3 {
            set.try_reserve(ReservationRequest {
                model_id: "m",
                cost_mb: 1000,
                pinned: false,
                last_used_ms: last_used.clone(),
                in_flight: in_flight.clone(),
            })
            .unwrap();
        }
        assert_eq!(set.total_reserved_mb(), 1000);
    }

    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering as AOrd};

    fn mk_entry_req<'a>(
        model_id: &'a str,
        cost_mb: u64,
        pinned: bool,
        last_used_ms: u64,
        in_flight_val: u32,
    ) -> ReservationRequest<'a> {
        ReservationRequest {
            model_id,
            cost_mb,
            pinned,
            last_used_ms: Arc::new(AtomicU64::new(last_used_ms)),
            in_flight: Arc::new(AtomicU32::new(in_flight_val)),
        }
    }

    #[test]
    fn pick_eviction_candidates_returns_lru_first() {
        let set = ResidentSet::new(10_000);
        set.try_reserve(mk_entry_req("a", 3000, false, 300, 0)).unwrap();
        set.try_reserve(mk_entry_req("b", 3000, false, 100, 0)).unwrap();
        set.try_reserve(mk_entry_req("c", 3000, false, 200, 0)).unwrap();
        let victims = set.pick_eviction_candidates(3000).unwrap();
        assert_eq!(victims, vec!["b".to_string()]);
    }

    #[test]
    fn pick_eviction_candidates_skips_pinned() {
        let set = ResidentSet::new(10_000);
        set.try_reserve(mk_entry_req("pinned_old", 3000, true, 100, 0)).unwrap();
        set.try_reserve(mk_entry_req("unpinned_new", 3000, false, 500, 0)).unwrap();
        let victims = set.pick_eviction_candidates(3000).unwrap();
        assert_eq!(victims, vec!["unpinned_new".to_string()]);
    }

    #[test]
    fn pick_eviction_candidates_skips_busy() {
        let set = ResidentSet::new(10_000);
        set.try_reserve(mk_entry_req("busy_old", 3000, false, 100, 1)).unwrap();
        set.try_reserve(mk_entry_req("idle_new", 3000, false, 500, 0)).unwrap();
        let victims = set.pick_eviction_candidates(3000).unwrap();
        assert_eq!(victims, vec!["idle_new".to_string()]);
    }

    #[test]
    fn pick_eviction_candidates_returns_none_when_insufficient() {
        let set = ResidentSet::new(10_000);
        set.try_reserve(mk_entry_req("pinned", 3000, true, 100, 0)).unwrap();
        set.try_reserve(mk_entry_req("busy", 3000, false, 200, 1)).unwrap();
        assert!(set.pick_eviction_candidates(3000).is_none());
    }

    #[test]
    fn pick_eviction_candidates_accumulates_multiple_victims() {
        let set = ResidentSet::new(10_000);
        set.try_reserve(mk_entry_req("a", 2000, false, 100, 0)).unwrap();
        set.try_reserve(mk_entry_req("b", 2000, false, 200, 0)).unwrap();
        set.try_reserve(mk_entry_req("c", 2000, false, 300, 0)).unwrap();
        // Need 3500 → evict a+b (oldest two), sum=4000.
        let victims = set.pick_eviction_candidates(3500).unwrap();
        assert_eq!(victims.len(), 2);
        assert!(victims.contains(&"a".to_string()));
        assert!(victims.contains(&"b".to_string()));
        assert!(!victims.contains(&"c".to_string()));
    }

    #[test]
    fn pick_eviction_candidates_reads_in_flight_live() {
        // Verifies in_flight is checked at call time, not at reserve time.
        let set = ResidentSet::new(10_000);
        let in_flight_a = Arc::new(AtomicU32::new(0));
        let in_flight_b = Arc::new(AtomicU32::new(0));
        set.try_reserve(ReservationRequest {
            model_id: "a",
            cost_mb: 3000,
            pinned: false,
            last_used_ms: Arc::new(AtomicU64::new(100)),
            in_flight: in_flight_a.clone(),
        })
        .unwrap();
        set.try_reserve(ReservationRequest {
            model_id: "b",
            cost_mb: 3000,
            pinned: false,
            last_used_ms: Arc::new(AtomicU64::new(500)),
            in_flight: in_flight_b.clone(),
        })
        .unwrap();

        // 'a' is LRU but becomes busy after reservation — should be skipped.
        in_flight_a.store(1, AOrd::Release);
        let victims = set.pick_eviction_candidates(3000).unwrap();
        assert_eq!(victims, vec!["b".to_string()]);
    }

    // ── Scheduler tests ──────────────────────────────────────────────────────

    use super::{Scheduler, SchedulerError};

    fn mk_ctx<'a>(
        model_id: &'a str,
        pinned: bool,
    ) -> super::ReservationContext<'a> {
        super::ReservationContext {
            model_id,
            pinned,
            last_used_ms: Arc::new(AtomicU64::new(0)),
            in_flight: Arc::new(AtomicU32::new(0)),
        }
    }

    #[test]
    fn scheduler_picks_device_with_most_free_budget() {
        let sched = Scheduler::new(vec![5000, 8000]);
        assert_eq!(sched.pick_most_free_device(), Some(1));
    }

    #[test]
    fn scheduler_picks_least_loaded_across_unequal_budgets() {
        // Device 0: budget 20000, reserve 19000 → free 1000.
        // Device 1: budget 10000, reserve 2000 → free 8000.
        let sched = Scheduler::new(vec![20000, 10000]);
        sched
            .set(0)
            .unwrap()
            .try_reserve(ReservationRequest {
                model_id: "big",
                cost_mb: 19000,
                pinned: false,
                last_used_ms: Arc::new(AtomicU64::new(0)),
                in_flight: Arc::new(AtomicU32::new(0)),
            })
            .unwrap();
        sched
            .set(1)
            .unwrap()
            .try_reserve(ReservationRequest {
                model_id: "small",
                cost_mb: 2000,
                pinned: false,
                last_used_ms: Arc::new(AtomicU64::new(0)),
                in_flight: Arc::new(AtomicU32::new(0)),
            })
            .unwrap();
        assert_eq!(sched.pick_most_free_device(), Some(1));
    }

    #[test]
    fn scheduler_returns_none_when_no_devices() {
        let sched = Scheduler::new(vec![]);
        assert_eq!(sched.pick_most_free_device(), None);
        assert_eq!(sched.device_count(), 0);
    }

    #[test]
    fn scheduler_try_reserve_split_succeeds_across_devices() {
        let sched = Scheduler::new(vec![10000, 10000]);
        let per_device = vec![(0, 5000), (1, 5000)];
        sched.try_reserve_split(&per_device, mk_ctx("big", false)).unwrap();
        assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 5000);
        assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 5000);
    }

    #[test]
    fn scheduler_try_reserve_split_rollback_on_partial_failure() {
        // Fill device 1 so the second reserve fails.
        let sched = Scheduler::new(vec![10000, 1000]);
        sched
            .set(1)
            .unwrap()
            .try_reserve(ReservationRequest {
                model_id: "filler",
                cost_mb: 900,
                pinned: false,
                last_used_ms: Arc::new(AtomicU64::new(0)),
                in_flight: Arc::new(AtomicU32::new(0)),
            })
            .unwrap();

        // Try to reserve 5000 on 0 + 5000 on 1. First succeeds, second fails.
        let per_device = vec![(0, 5000), (1, 5000)];
        let err = sched
            .try_reserve_split(&per_device, mk_ctx("big", false))
            .unwrap_err();
        assert!(matches!(err, SchedulerError::OverBudget { gpu_id: 1, .. }));

        // Device 0 should be back to 0 reserved (rolled back).
        assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 0);
        // Device 1 still has the filler.
        assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 900);
    }

    #[test]
    fn scheduler_try_reserve_split_unknown_gpu() {
        let sched = Scheduler::new(vec![10000]);
        let per_device = vec![(5, 1000)]; // gpu 5 doesn't exist
        let err = sched
            .try_reserve_split(&per_device, mk_ctx("m", false))
            .unwrap_err();
        assert!(matches!(err, SchedulerError::UnknownGpu { gpu_id: 5 }));
    }

    #[test]
    fn scheduler_release_iterates_gpu_ids() {
        let sched = Scheduler::new(vec![10000, 10000]);
        sched.try_reserve_split(&[(0, 5000), (1, 5000)], mk_ctx("big", false)).unwrap();
        sched.release("big", &[0, 1]);
        assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 0);
        assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 0);
    }
}
