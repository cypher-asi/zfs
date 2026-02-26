use std::fmt;

/// Errors that can occur during Grid cryptographic operations.
#[derive(Debug)]
pub enum CryptoError {
    /// Sealed ciphertext is too short to contain nonce + tag.
    CiphertextTooShort { len: usize, min: usize },
    /// AEAD encryption failed.
    EncryptionFailed,
    /// AEAD decryption failed (wrong key, corrupted data, or AAD mismatch).
    DecryptionFailed,
    /// HKDF expand failed during key derivation.
    HkdfExpandFailed,
    /// Padding or unpadding failed.
    PaddingError(String),
    /// Error from the underlying `zid` crate.
    Neural(zid::CryptoError),
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CiphertextTooShort { len, min } => {
                write!(f, "ciphertext too short: {len} bytes, minimum {min}")
            }
            Self::EncryptionFailed => f.write_str("AEAD encryption failed"),
            Self::DecryptionFailed => f.write_str("AEAD decryption failed"),
            Self::HkdfExpandFailed => f.write_str("HKDF expand failed"),
            Self::PaddingError(msg) => write!(f, "padding error: {msg}"),
            Self::Neural(e) => write!(f, "neural: {e}"),
        }
    }
}

impl std::error::Error for CryptoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Neural(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

impl From<zid::CryptoError> for CryptoError {
    fn from(e: zid::CryptoError) -> Self {
        Self::Neural(e)
    }
}
