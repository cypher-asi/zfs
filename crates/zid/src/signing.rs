use ml_dsa::{KeyGen, MlDsa65};
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

/// PQ-Hybrid signature: always contains both Ed25519 and ML-DSA-65 components.
/// Both must verify for the signature to be considered valid.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HybridSignature {
    #[serde(with = "serde_bytes")]
    pub ed25519: [u8; 64],
    #[serde(with = "serde_bytes")]
    pub ml_dsa: Vec<u8>,
}

impl HybridSignature {
    pub const ED25519_LEN: usize = 64;
    pub const ML_DSA_65_LEN: usize = 3_309;

    /// Serialize to bytes: ed25519 (64) || ml_dsa (3309).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::ED25519_LEN + self.ml_dsa.len());
        out.extend_from_slice(&self.ed25519);
        out.extend_from_slice(&self.ml_dsa);
        out
    }

    /// Deserialize from bytes: ed25519 (64) || ml_dsa (remainder).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < Self::ED25519_LEN + Self::ML_DSA_65_LEN {
            return Err(CryptoError::InvalidKeyLength {
                expected: Self::ED25519_LEN + Self::ML_DSA_65_LEN,
                got: bytes.len(),
            });
        }
        let mut ed25519 = [0u8; 64];
        ed25519.copy_from_slice(&bytes[..64]);
        let ml_dsa = bytes[64..].to_vec();
        Ok(Self { ed25519, ml_dsa })
    }
}

/// Identity Signing Key — Ed25519 + ML-DSA-65 hybrid.
///
/// Derived from a NeuralKey via HKDF. Produces hybrid signatures that
/// require both classical and post-quantum components to verify.
#[derive(ZeroizeOnDrop)]
pub struct IdentitySigningKey {
    #[zeroize(skip)]
    ed25519: ed25519_dalek::SigningKey,
    #[zeroize(skip)]
    ml_dsa_signing: ml_dsa::SigningKey<MlDsa65>,
    #[zeroize(skip)]
    ml_dsa_verifying: ml_dsa::VerifyingKey<MlDsa65>,
}

impl IdentitySigningKey {
    pub(crate) fn from_seeds(ed25519_seed: [u8; 32], ml_dsa_seed: [u8; 32]) -> Self {
        let ed25519 = ed25519_dalek::SigningKey::from_bytes(&ed25519_seed);

        let ml_dsa_b32 = arr_from_bytes(ml_dsa_seed);
        let kp = MlDsa65::key_gen_internal(&ml_dsa_b32);
        let ml_dsa_verifying = kp.verifying_key().clone();
        let ml_dsa_signing = kp.signing_key().clone();

        Self {
            ed25519,
            ml_dsa_signing,
            ml_dsa_verifying,
        }
    }

    /// Produce a hybrid signature over `msg`.
    pub fn sign(&self, msg: &[u8]) -> HybridSignature {
        use ed25519_dalek::Signer as _;
        use ml_dsa::signature::SignatureEncoding as _;

        let ed_sig = self.ed25519.sign(msg);
        let pq_sig: ml_dsa::Signature<MlDsa65> =
            ml_dsa::signature::Signer::sign(&self.ml_dsa_signing, msg);

        HybridSignature {
            ed25519: ed_sig.to_bytes(),
            ml_dsa: pq_sig.to_bytes().to_vec(),
        }
    }

    /// Extract the corresponding verifying key.
    pub fn verifying_key(&self) -> IdentityVerifyingKey {
        IdentityVerifyingKey {
            ed25519: self.ed25519.verifying_key(),
            ml_dsa: self.ml_dsa_verifying.clone(),
        }
    }

    /// Access the raw Ed25519 public key bytes (for DID encoding).
    pub fn ed25519_public_bytes(&self) -> [u8; 32] {
        self.ed25519.verifying_key().to_bytes()
    }
}

impl core::fmt::Debug for IdentitySigningKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IdentitySigningKey")
            .field("ed25519_public", &self.ed25519.verifying_key())
            .finish_non_exhaustive()
    }
}

/// Identity Verifying Key — Ed25519 + ML-DSA-65 public keys.
#[derive(Debug, Clone)]
pub struct IdentityVerifyingKey {
    ed25519: ed25519_dalek::VerifyingKey,
    ml_dsa: ml_dsa::VerifyingKey<MlDsa65>,
}

impl IdentityVerifyingKey {
    /// Verify a hybrid signature: both Ed25519 and ML-DSA-65 must pass.
    pub fn verify(&self, msg: &[u8], sig: &HybridSignature) -> Result<(), CryptoError> {
        use ed25519_dalek::Verifier as _;

        let ed_sig = ed25519_dalek::Signature::from_bytes(&sig.ed25519);
        self.ed25519
            .verify(msg, &ed_sig)
            .map_err(|_| CryptoError::Ed25519VerifyFailed)?;

        let pq_sig = <ml_dsa::Signature<MlDsa65>>::try_from(sig.ml_dsa.as_slice())
            .map_err(|_| CryptoError::MlDsaVerifyFailed)?;
        ml_dsa::signature::Verifier::verify(&self.ml_dsa, msg, &pq_sig)
            .map_err(|_| CryptoError::MlDsaVerifyFailed)?;

        Ok(())
    }

    /// Access the raw Ed25519 public key bytes (for DID encoding).
    pub fn ed25519_bytes(&self) -> [u8; 32] {
        self.ed25519.to_bytes()
    }
}

// --- Helpers ---

/// Construct a `B32` (`Array<u8, U32>`) from a `[u8; 32]`.
pub(crate) fn arr_from_bytes(bytes: [u8; 32]) -> ml_dsa::B32 {
    let mut arr = ml_dsa::B32::default();
    arr.copy_from_slice(&bytes);
    arr
}
