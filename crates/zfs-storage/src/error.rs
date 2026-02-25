use thiserror::Error;

/// Errors from the ZFS storage layer.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("rocksdb error: {0}")]
    RocksDb(#[from] rocksdb::Error),

    #[error("encode error: {0}")]
    Encode(String),

    #[error("decode error: {0}")]
    Decode(String),

    #[error("storage full: {reason}")]
    Full { reason: String },

    #[error("column family not found: {0}")]
    CfNotFound(String),

    #[error("batch too large: {0}")]
    BatchTooLarge(String),
}

impl From<StorageError> for zfs_core::ZfsError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::Full { .. } => zfs_core::ZfsError::StorageFull,
            StorageError::Encode(msg) => zfs_core::ZfsError::Encode(msg),
            StorageError::Decode(msg) => zfs_core::ZfsError::Decode(msg),
            StorageError::BatchTooLarge(msg) => zfs_core::ZfsError::InvalidPayload(msg),
            other => zfs_core::ZfsError::Other(other.to_string()),
        }
    }
}
