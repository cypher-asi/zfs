use thiserror::Error;

#[derive(Debug, Error)]
pub enum Groth16Error {
    #[error("setup failed: {0}")]
    SetupFailed(String),

    #[error("proving failed: {0}")]
    ProvingFailed(String),

    #[error("verification failed: {0}")]
    VerificationFailed(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("invalid bucket size: {size}")]
    InvalidBucketSize { size: u32 },

    #[error("plaintext too large for bucket: {len} > {max}")]
    PlaintextTooLarge { len: usize, max: usize },
}
