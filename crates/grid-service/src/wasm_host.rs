// WASM Service Host — loads .wasm components and bridges them to the native
// Service trait. Gated behind the `wasm` feature flag (requires `wasmtime`).
//
// This module defines the interface and a placeholder implementation. The full
// wasmtime integration will be enabled once the `wasm` Cargo feature is active.

use std::path::Path;
use std::time::Duration;

use crate::descriptor::ServiceDescriptor;
use crate::error::ServiceError;

/// Resource limits for WASM service instances.
#[derive(Debug, Clone)]
pub struct WasmResourceLimits {
    /// Max fuel (CPU instructions) per request.
    pub max_fuel_per_request: u64,
    /// Max memory the WASM instance can allocate.
    pub max_memory_bytes: usize,
    /// Max wall-clock time per request.
    pub max_request_duration: Duration,
}

impl Default for WasmResourceLimits {
    fn default() -> Self {
        Self {
            max_fuel_per_request: 1_000_000_000,
            max_memory_bytes: 64 * 1024 * 1024,
            max_request_duration: Duration::from_secs(30),
        }
    }
}

/// Configuration for loading a WASM service module.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WasmServiceConfig {
    pub path: std::path::PathBuf,
    pub enabled: bool,
}

/// Load a WASM service component descriptor from disk.
///
/// Currently returns an error because the WASM runtime requires the `wasm`
/// feature flag (which brings in `wasmtime`). The full `WasmServiceHost`
/// (which implements the `Service` trait) will be available with that flag.
pub fn load_descriptor(_path: &Path) -> Result<ServiceDescriptor, ServiceError> {
    Err(ServiceError::Other(
        "WASM runtime not enabled — compile with `--features wasm`".into(),
    ))
}
