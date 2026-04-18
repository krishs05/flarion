use std::sync::Arc;

use crate::admin::state::AdminState;
use crate::admin::types::Gpu;

/// Best-effort GPU snapshot from NVML. On CPU-only builds or when NVML isn't
/// available (no driver, virtualization, Apple Silicon, etc.) returns an
/// empty Vec — the admin API contract is that `gpus` can be empty.
///
/// `reserved_mb` is derived from NVML's `total - free` figure, which reflects
/// ALL processes using the device, not just Flarion. A later task can
/// substitute per-model reservations from the Scheduler; for now, NVML view
/// is a reasonable first approximation.
pub fn gpu_snapshot(_state: &Arc<AdminState>) -> Vec<Gpu> {
    #[cfg(feature = "cuda")]
    {
        use nvml_wrapper::Nvml;
        if let Ok(nvml) = Nvml::init() {
            let count = nvml.device_count().unwrap_or(0);
            return (0..count).filter_map(|i| {
                let dev = nvml.device_by_index(i).ok()?;
                let mem = dev.memory_info().ok()?;
                Some(Gpu {
                    id: i,
                    name: dev.name().unwrap_or_else(|_| format!("GPU {i}")),
                    budget_mb: mem.total / 1_048_576,
                    reserved_mb: (mem.total - mem.free) / 1_048_576,
                    free_mb: mem.free / 1_048_576,
                    models: Vec::new(),
                })
            }).collect();
        }
    }
    Vec::new()
}
