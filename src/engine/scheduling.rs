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
}
