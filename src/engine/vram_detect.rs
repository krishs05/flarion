//! VRAM detection via NVIDIA Management Library (NVML).
//!
//! Used by `ServerConfig::resolve_vram_budget_mb` when `vram_budget_mb = "auto"`
//! is set. On hosts without an NVIDIA driver, returns `VramDetectError` and the
//! startup code surfaces `ConfigError::VramAutoDetectFailed` with a remediation
//! hint directing the operator to an explicit budget value.

#[derive(Debug, Clone)]
pub struct VramInfo {
    pub device_index: u32,
    pub total_mb: u64,
    pub free_mb: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum VramDetectError {
    #[error("NVML init failed: {0}")]
    NvmlInit(#[source] nvml_wrapper::error::NvmlError),
    #[error("no CUDA devices detected")]
    NoDevices,
    #[error("NVML query failed: {0}")]
    QueryFailed(#[source] nvml_wrapper::error::NvmlError),
}

/// Query every CUDA device visible to NVML. Returns a `Vec<VramInfo>`
/// in device-index order. On hosts without an NVIDIA driver, returns
/// `VramDetectError::NvmlInit`.
pub fn detect_all_devices() -> Result<Vec<VramInfo>, VramDetectError> {
    let nvml = nvml_wrapper::Nvml::init().map_err(VramDetectError::NvmlInit)?;
    let count = nvml.device_count().map_err(VramDetectError::QueryFailed)?;
    if count == 0 {
        return Err(VramDetectError::NoDevices);
    }
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let dev = nvml
            .device_by_index(i)
            .map_err(VramDetectError::QueryFailed)?;
        let mem = dev.memory_info().map_err(VramDetectError::QueryFailed)?;
        out.push(VramInfo {
            device_index: i,
            total_mb: mem.total / (1024 * 1024),
            free_mb: mem.free / (1024 * 1024),
        });
    }
    Ok(out)
}

/// Backward-compatible single-device accessor. Returns the first device
/// from `detect_all_devices`.
pub fn detect_device_zero() -> Result<VramInfo, VramDetectError> {
    detect_all_devices()?
        .into_iter()
        .next()
        .ok_or(VramDetectError::NoDevices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_device_zero_delegates_to_all_devices_first() {
        // This test exists only to assert that detect_device_zero is
        // still exported and returns the same VramInfo shape. It calls
        // NVML — on non-NVIDIA CI the Err path is exercised instead.
        let res = detect_device_zero();
        if let Ok(info) = res {
            assert_eq!(info.device_index, 0);
        }
    }
}
