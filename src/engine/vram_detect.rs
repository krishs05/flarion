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

pub fn detect_device_zero() -> Result<VramInfo, VramDetectError> {
    let nvml = nvml_wrapper::Nvml::init().map_err(VramDetectError::NvmlInit)?;
    let count = nvml.device_count().map_err(VramDetectError::QueryFailed)?;
    if count == 0 {
        return Err(VramDetectError::NoDevices);
    }
    let dev = nvml.device_by_index(0).map_err(VramDetectError::QueryFailed)?;
    let mem = dev.memory_info().map_err(VramDetectError::QueryFailed)?;
    Ok(VramInfo {
        device_index: 0,
        total_mb: mem.total / (1024 * 1024),
        free_mb: mem.free / (1024 * 1024),
    })
}
