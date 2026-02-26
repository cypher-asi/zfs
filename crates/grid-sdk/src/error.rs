use thiserror::Error;

/// Errors from the Grid SDK.
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("crypto error: {0}")]
    Crypto(#[from] grid_crypto::CryptoError),

    #[error("network error: {0}")]
    Network(#[from] grid_net::NetworkError),

    #[error("core error: {0}")]
    Core(#[from] grid_core::GridError),

    #[error("neural error: {0}")]
    Neural(#[from] zid::CryptoError),

    #[error("proof error: {0}")]
    Proof(#[from] grid_proof_groth16::Groth16Error),

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
