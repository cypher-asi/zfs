use thiserror::Error;

/// Errors from the ZFS SDK.
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("crypto error: {0}")]
    Crypto(#[from] zfs_crypto::CryptoError),

    #[error("network error: {0}")]
    Network(#[from] zfs_net::NetworkError),

    #[error("core error: {0}")]
    Core(#[from] zfs_core::ZfsError),

    #[error("neural error: {0}")]
    Neural(#[from] zero_neural::CryptoError),

    #[error("proof error: {0}")]
    Proof(#[from] zfs_proof_groth16::Groth16Error),

    #[error("no peers available for upload")]
    NoPeers,

    #[error("store rejected: {0}")]
    StoreRejected(String),

    #[error("upload failed: succeeded on {successes}/{required} zodes")]
    InsufficientReplication { successes: usize, required: usize },

    #[error("fetch not found")]
    NotFound,

    #[error("timeout: {0}")]
    Timeout(String),

    #[error("{0}")]
    Other(String),
}
