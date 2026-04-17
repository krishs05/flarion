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
        use std::sync::atomic::AtomicU32;
        use std::sync::atomic::AtomicU64;

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
    }

    #[test]
    fn reserve_request_idempotent_for_same_id() {
        use std::sync::atomic::AtomicU32;
        use std::sync::atomic::AtomicU64;

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
}
