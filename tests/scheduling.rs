//! Phase 2f integration tests: lazy loading + VRAM budget.
//!
//! These tests exercise config validation and startup-time budget
//! accounting. End-to-end "lazy model loads on first request" needs a
//! real GGUF and is covered in the manual verification plan.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::Arc;

use flarion::config::{BackendType, FlarionConfig, ModelConfig, VramBudgetSetting};
use flarion::engine::scheduling::{ReservationRequest, ResidentSet};

// Helper: build a sparse tempfile of `size_mb` MB.
fn make_fake_gguf(dir: &std::path::Path, name: &str, size_mb: u64) -> PathBuf {
    let path = dir.join(name);
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(size_mb * 1024 * 1024).unwrap();
    drop(f);
    path
}

fn local_model(id: &str, path: PathBuf, lazy: bool) -> ModelConfig {
    ModelConfig {
        id: id.into(),
        backend: BackendType::Local,
        path: Some(path),
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
        lazy,
        vram_mb: None,
        pin: false,
        gpus: vec![],
    }
}

#[test]
fn lazy_models_not_loaded_at_startup_via_resident_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_fake_gguf(dir.path(), "m.gguf", 100);

    let mut cfg = FlarionConfig::default();
    cfg.server.host = "127.0.0.1".into();
    cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(1000);
    cfg.models = vec![local_model("m", path, true)];
    cfg.validate().expect("lazy model should validate");

    // Simulate startup: construct ResidentSet, iterate models, skip lazy.
    let resident_set = ResidentSet::new(1000);
    for m in &cfg.models {
        if m.backend == BackendType::Local && !m.lazy {
            let path = m.path.as_ref().unwrap();
            let est = flarion::engine::scheduling::estimate_vram_mb(path, m.vram_mb).unwrap();
            resident_set
                .try_reserve(ReservationRequest {
                    model_id: &m.id,
                    cost_mb: est,
                    pinned: false,
                    last_used_ms: Arc::new(AtomicU64::new(0)),
                    in_flight: Arc::new(AtomicU32::new(0)),
                })
                .unwrap();
        }
    }
    // Lazy model was skipped.
    assert_eq!(resident_set.total_reserved_mb(), 0);
}

#[test]
fn eager_model_reserves_budget_at_startup_via_resident_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = make_fake_gguf(dir.path(), "m.gguf", 100);

    let mut cfg = FlarionConfig::default();
    cfg.server.host = "127.0.0.1".into();
    cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(1000);
    cfg.models = vec![local_model("m", path, false)];
    cfg.validate().expect("eager model should validate");

    let resident_set = ResidentSet::new(1000);
    for m in &cfg.models {
        if m.backend == BackendType::Local && !m.lazy {
            let path = m.path.as_ref().unwrap();
            let est = flarion::engine::scheduling::estimate_vram_mb(path, m.vram_mb).unwrap();
            resident_set
                .try_reserve(ReservationRequest {
                    model_id: &m.id,
                    cost_mb: est,
                    pinned: false,
                    last_used_ms: Arc::new(AtomicU64::new(0)),
                    in_flight: Arc::new(AtomicU32::new(0)),
                })
                .unwrap();
        }
    }
    // 100MB file * 1.2 = 120MB reserved.
    let reserved = resident_set.total_reserved_mb();
    assert!((119..=121).contains(&reserved), "reserved={reserved}");
}

#[test]
fn overbudget_eager_config_fails_validation() {
    let dir = tempfile::tempdir().unwrap();
    let path_a = make_fake_gguf(dir.path(), "a.gguf", 200);
    let path_b = make_fake_gguf(dir.path(), "b.gguf", 200);
    let path_c = make_fake_gguf(dir.path(), "c.gguf", 200);

    let mut cfg = FlarionConfig::default();
    cfg.server.host = "127.0.0.1".into();
    cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(500);
    // Assign all models to gpu 0 so per-device check applies.
    let mut a = local_model("a", path_a, false);
    a.gpus = vec![0];
    let mut b = local_model("b", path_b, false);
    b.gpus = vec![0];
    let mut c = local_model("c", path_c, false);
    c.gpus = vec![0];
    cfg.models = vec![a, b, c];
    // Each estimate ~240MB, total ~720MB > 500MB budget on gpu 0.
    let err = cfg.validate().unwrap_err();
    assert!(
        format!("{err}").contains("exceeds budget="),
        "got: {err}"
    );
}

#[test]
fn pick_eviction_candidates_excludes_pinned_across_integration() {
    let set = ResidentSet::new(10_000);
    set.try_reserve(ReservationRequest {
        model_id: "pinned",
        cost_mb: 5000,
        pinned: true,
        last_used_ms: Arc::new(AtomicU64::new(100)),
        in_flight: Arc::new(AtomicU32::new(0)),
    })
    .unwrap();
    set.try_reserve(ReservationRequest {
        model_id: "unpinned",
        cost_mb: 4000,
        pinned: false,
        last_used_ms: Arc::new(AtomicU64::new(500)),
        in_flight: Arc::new(AtomicU32::new(0)),
    })
    .unwrap();
    // Need 3000; pinned is older but must not be chosen.
    let victims = set.pick_eviction_candidates(3000).unwrap();
    assert_eq!(victims, vec!["unpinned".to_string()]);
}

#[test]
fn pick_eviction_candidates_respects_budget_when_all_busy() {
    let set = ResidentSet::new(10_000);
    set.try_reserve(ReservationRequest {
        model_id: "a",
        cost_mb: 5000,
        pinned: false,
        last_used_ms: Arc::new(AtomicU64::new(100)),
        in_flight: Arc::new(AtomicU32::new(1)), // busy
    })
    .unwrap();
    assert!(set.pick_eviction_candidates(5000).is_none());
}

#[test]
fn overbudget_pinned_config_fails_validation() {
    use flarion::config::{BackendType, FlarionConfig, ModelConfig};
    use std::path::PathBuf;

    let dir = tempfile::tempdir().unwrap();
    let p = |n: &str| {
        let path = dir.path().join(n);
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(300 * 1024 * 1024).unwrap();
        drop(f);
        path
    };

    fn lazy_pinned(id: &str, path: PathBuf) -> ModelConfig {
        ModelConfig {
            id: id.into(),
            backend: BackendType::Local,
            path: Some(path),
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
            pin: true,
            gpus: vec![0],
        }
    }

    let mut cfg = FlarionConfig::default();
    cfg.server.host = "127.0.0.1".into();
    cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(500);
    cfg.models = vec![
        lazy_pinned("a", p("a.gguf")),
        lazy_pinned("b", p("b.gguf")),
    ];
    let err = cfg.validate().unwrap_err();
    assert!(
        format!("{err}").contains("pinned local models on gpu"),
        "got {err}"
    );
}

#[tokio::test]
async fn evictor_trait_dispatches_to_backend_unload() {
    // Verifies BackendRegistry::unload calls through to the matched backend.
    use flarion::engine::backend::{Evictor, InferenceBackend};
    use flarion::engine::registry::BackendRegistry;
    use flarion::engine::testing::MockBackend;

    let mut registry = BackendRegistry::new();
    registry.insert(
        "m".into(),
        Arc::new(MockBackend::succeeding("m", "hi")) as Arc<dyn InferenceBackend>,
    );
    let registry = Arc::new(registry);
    // MockBackend uses the default no-op unload, which returns Ok.
    registry.unload("m").await.unwrap();

    // Unknown model → ModelNotFound.
    let err = registry.unload("nope").await.unwrap_err();
    assert!(matches!(err, flarion::error::EngineError::ModelNotFound(_)));
}

#[test]
fn integration_scheduler_best_fit_prefers_emptier_device() {
    use flarion::engine::scheduling::{ReservationRequest, Scheduler};
    use std::sync::atomic::{AtomicU32, AtomicU64};
    use std::sync::Arc;

    let sched = Scheduler::new(vec![10_000, 10_000]);
    // Fill gpu 0 to 9000 MB. gpu 1 stays empty.
    sched
        .set(0)
        .unwrap()
        .try_reserve(ReservationRequest {
            model_id: "filler",
            cost_mb: 9000,
            pinned: false,
            last_used_ms: Arc::new(AtomicU64::new(0)),
            in_flight: Arc::new(AtomicU32::new(0)),
        })
        .unwrap();
    assert_eq!(sched.pick_most_free_device(), Some(1));
}

#[test]
fn integration_scheduler_split_reservation_rolls_back_on_failure() {
    use flarion::engine::scheduling::{
        ReservationContext, ReservationRequest, Scheduler, SchedulerError,
    };
    use std::sync::atomic::{AtomicU32, AtomicU64};
    use std::sync::Arc;

    let sched = Scheduler::new(vec![10_000, 1_000]);
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

    let ctx = ReservationContext {
        model_id: "big",
        pinned: false,
        last_used_ms: Arc::new(AtomicU64::new(0)),
        in_flight: Arc::new(AtomicU32::new(0)),
    };
    let err = sched
        .try_reserve_split(&[(0, 5000), (1, 500)], ctx)
        .unwrap_err();
    assert!(matches!(err, SchedulerError::OverBudget { gpu_id: 1, .. }));

    // Rollback: gpu 0 back to 0.
    assert_eq!(sched.set(0).unwrap().total_reserved_mb(), 0);
    // Gpu 1 unchanged.
    assert_eq!(sched.set(1).unwrap().total_reserved_mb(), 900);
}

#[test]
fn integration_gpu_id_exceeds_declared_device_count_rejected() {
    use flarion::config::{BackendType, FlarionConfig, ModelConfig, VramBudgetSetting};

    let dir = tempfile::tempdir().unwrap();
    let path = {
        let p = dir.path().join("m.gguf");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(100 * 1024 * 1024).unwrap();
        drop(f);
        p
    };

    // The override references a gpu_id beyond the declared device count,
    // which resolve_vram_budgets (called inside validate) rejects with
    // VramOverrideUnknownGpu.
    let mut cfg = FlarionConfig::default();
    cfg.server.host = "127.0.0.1".into();
    cfg.server.vram_budget_mb = VramBudgetSetting::Fixed(1000);
    cfg.server.vram_budget_overrides.insert(5, 500);
    cfg.models = vec![ModelConfig {
        id: "m".into(),
        backend: BackendType::Local,
        path: Some(path),
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
        lazy: false,
        vram_mb: None,
        pin: false,
        gpus: vec![0],
    }];
    let err = cfg.validate().unwrap_err();
    assert!(
        format!("{err}").contains("vram_budget_overrides references gpu 5"),
        "got {err}"
    );
}
