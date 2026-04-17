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
use std::sync::{Arc, Mutex};

pub struct ResidentSet {
    inner: Mutex<Inner>,
    budget_mb: u64,
}

struct Inner {
    loaded: HashMap<String, u64>,
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
            .map(|inner| inner.loaded.values().sum())
            .unwrap_or(0)
    }

    /// Reserve `cost_mb` for `model_id`. Idempotent for the same id.
    /// Returns `OverBudget` if this reservation would exceed the budget.
    /// Zero-budget sets always succeed (scheduling disabled).
    pub fn try_reserve(&self, model_id: &str, cost_mb: u64) -> Result<(), ResidentError> {
        if self.budget_mb == 0 {
            return Ok(());
        }

        let mut inner = self.inner.lock().map_err(|_| ResidentError::Poisoned)?;

        if inner.loaded.contains_key(model_id) {
            return Ok(());
        }

        let current: u64 = inner.loaded.values().sum();
        if current.saturating_add(cost_mb) > self.budget_mb {
            return Err(ResidentError::OverBudget {
                model_id: model_id.to_string(),
                requested_mb: cost_mb,
                current_mb: current,
                budget_mb: self.budget_mb,
            });
        }

        inner.loaded.insert(model_id.to_string(), cost_mb);
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

    #[test]
    fn reserve_within_budget_succeeds() {
        let set = ResidentSet::new(4000);
        assert!(set.try_reserve("m", 1000).is_ok());
        assert_eq!(set.total_reserved_mb(), 1000);
    }

    #[test]
    fn reserve_exceeding_budget_returns_err() {
        let set = ResidentSet::new(4000);
        let err = set.try_reserve("m", 5000).unwrap_err();
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
        assert!(set.try_reserve("a", 1000).is_ok());
        assert!(set.try_reserve("b", 2000).is_ok());
        assert!(set.try_reserve("c", 2000).is_err());
        assert_eq!(set.total_reserved_mb(), 3000);
    }

    #[test]
    fn reserve_is_idempotent_for_same_id() {
        let set = ResidentSet::new(4000);
        assert!(set.try_reserve("m", 1000).is_ok());
        assert!(set.try_reserve("m", 999_999).is_ok());
        assert_eq!(set.total_reserved_mb(), 1000);
    }

    #[test]
    fn release_removes_reservation() {
        let set = ResidentSet::new(4000);
        set.try_reserve("m", 1000).unwrap();
        set.release("m");
        assert_eq!(set.total_reserved_mb(), 0);
        assert!(set.try_reserve("m", 2000).is_ok());
        assert_eq!(set.total_reserved_mb(), 2000);
    }

    #[test]
    fn zero_budget_means_disabled() {
        let set = ResidentSet::new(0);
        assert!(set.try_reserve("m", u64::MAX).is_ok());
        assert_eq!(set.total_reserved_mb(), 0);
        set.release("m");
    }

    #[test]
    fn overbudget_error_carries_detail() {
        let set = ResidentSet::new(4000);
        set.try_reserve("existing", 3000).unwrap();
        let err = set.try_reserve("new", 2000).unwrap_err();
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
}
