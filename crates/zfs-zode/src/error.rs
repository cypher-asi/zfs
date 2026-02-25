use thiserror::Error;

/// Errors from the Zode.
#[derive(Debug, Error)]
pub enum ZodeError {
    #[error("storage error: {0}")]
    Storage(#[from] zfs_storage::StorageError),

    #[error("network error: {0}")]
    Network(#[from] zfs_net::NetworkError),

    #[error("core error: {0}")]
    Core(#[from] zfs_core::ZfsError),

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
    pub fn to_error_code(&self) -> zfs_core::ErrorCode {
        match self {
            Self::Storage(zfs_storage::StorageError::Full { .. }) => {
                zfs_core::ErrorCode::StorageFull
            }
            Self::StorageFull(_) => zfs_core::ErrorCode::StorageFull,
            Self::PolicyReject(_) => zfs_core::ErrorCode::PolicyReject,
            Self::Core(e) => e
                .error_code()
                .unwrap_or(zfs_core::ErrorCode::InvalidPayload),
            _ => zfs_core::ErrorCode::InvalidPayload,
        }
    }
}
