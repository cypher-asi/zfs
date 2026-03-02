use thiserror::Error;

/// Errors from the Zode.
#[derive(Debug, Error)]
pub enum ZodeError {
    #[error("storage error: {0}")]
    Storage(#[from] grid_storage::StorageError),

    #[error("network error: {0}")]
    Network(#[from] grid_net::NetworkError),

    #[error("core error: {0}")]
    Core(#[from] grid_core::GridError),

    #[error("service error: {0}")]
    Service(#[from] grid_service::ServiceError),

    #[error("policy reject: {0}")]
    PolicyReject(String),

    #[error("storage full: {0}")]
    StorageFull(String),

    #[error("shutdown")]
    Shutdown,

    #[error("{0}")]
    Other(String),
}

impl ZodeError {
    /// Map to a wire-safe error code for protocol responses.
    pub fn to_error_code(&self) -> grid_core::ErrorCode {
        match self {
            Self::Storage(grid_storage::StorageError::Full { .. }) => {
                grid_core::ErrorCode::StorageFull
            }
            Self::StorageFull(_) => grid_core::ErrorCode::StorageFull,
            Self::PolicyReject(_) => grid_core::ErrorCode::PolicyReject,
            Self::Core(e) => e
                .error_code()
                .unwrap_or(grid_core::ErrorCode::InvalidPayload),
            _ => grid_core::ErrorCode::InvalidPayload,
        }
    }
}
