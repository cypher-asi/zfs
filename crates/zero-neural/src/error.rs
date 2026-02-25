use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("HKDF expand failed: output length invalid")]
    HkdfExpandFailed,

    #[error("Ed25519 signature verification failed")]
    Ed25519VerifyFailed,

    #[error("ML-DSA-65 signature verification failed")]
    MlDsaVerifyFailed,

    #[error("hybrid signature verification failed: {0}")]
    HybridVerifyFailed(&'static str),

    #[error("ML-KEM-768 decapsulation failed")]
    MlKemDecapFailed,

    #[error("X25519 key agreement produced all-zero shared secret")]
    X25519ZeroSharedSecret,

    #[error("invalid DID key: {0}")]
    InvalidDid(String),

    #[error("invalid key length: expected {expected}, got {got}")]
    InvalidKeyLength { expected: usize, got: usize },

    #[error("invalid ciphertext length: expected {expected}, got {got}")]
    InvalidCiphertextLength { expected: usize, got: usize },

    #[error("Shamir split failed: {0}")]
    ShamirSplitFailed(String),

    #[error("Shamir recovery failed: {0}")]
    ShamirRecoveryFailed(String),
}
