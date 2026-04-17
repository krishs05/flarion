//! Phase 2f integration tests: lazy loading + VRAM budget.
//!
//! These tests exercise config validation and startup-time budget
//! accounting. End-to-end "lazy model loads on first request" needs a
//! real GGUF and is covered in the manual verification plan.

use std::path::PathBuf;

use flarion::config::{BackendType, FlarionConfig, ModelConfig, VramBudgetSetting};
use flarion::engine::scheduling::ResidentSet;

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
            resident_set.try_reserve(&m.id, est).unwrap();
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
            resident_set.try_reserve(&m.id, est).unwrap();
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
    cfg.models = vec![
        local_model("a", path_a, false),
        local_model("b", path_b, false),
        local_model("c", path_c, false),
    ];
    // Each estimate ~240MB, total ~720MB > 500MB budget.
    let err = cfg.validate().unwrap_err();
    assert!(
        format!("{err}").contains("exceeds vram_budget_mb"),
        "got: {err}"
    );
}
