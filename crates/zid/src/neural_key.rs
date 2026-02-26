use rand_core::{CryptoRng, RngCore};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A 256-bit root secret from which all identity and machine keys are derived.
///
/// Generated via CSPRNG. Must be stored securely (e.g. encrypted at rest,
/// Shamir-split for recovery). Zeroized on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub(crate) struct NeuralKey([u8; 32]);

impl NeuralKey {
    /// Generate a new NeuralKey from a cryptographically secure RNG.
    pub(crate) fn generate(rng: &mut (impl RngCore + CryptoRng)) -> Self {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Access the raw key material (for HKDF derivation only).
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Copy the raw key material (needed by Shamir split).
    pub(crate) fn to_bytes(&self) -> [u8; 32] {
        self.0
    }

    /// Reconstruct a NeuralKey from raw bytes (e.g. after Shamir recovery).
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Debug for NeuralKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("NeuralKey").field(&"[REDACTED]").finish()
    }
}
