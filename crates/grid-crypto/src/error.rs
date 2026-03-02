use thiserror::Error;

/// Errors that can occur during Grid cryptographic operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Sealed ciphertext is too short to contain nonce + tag.
    #[error("ciphertext too short: {len} bytes, minimum {min}")]
    CiphertextTooShort { len: usize, min: usize },
    /// AEAD encryption failed.
    #[error("AEAD encryption failed")]
    EncryptionFailed,
    /// AEAD decryption failed (wrong key, corrupted data, or AAD mismatch).
    #[error("AEAD decryption failed")]
    DecryptionFailed,
    /// HKDF expand failed during key derivation.
    #[error("HKDF expand failed")]
    HkdfExpandFailed,
    /// Padding or unpadding failed.
    #[error("padding error: {0}")]
    PaddingError(String),
    /// Error from the underlying `zid` crate.
    #[error("neural: {0}")]
    Neural(#[from] zid::CryptoError),
}
