use bitflags::bitflags;
use ml_dsa::{KeyGen, MlDsa65};
use ml_kem::{KemCore, MlKem768};

use crate::error::CryptoError;
use crate::signing::{arr_from_bytes, HybridSignature};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MachineKeyCapabilities: u8 {
        const SIGN    = 0x01;
        const ENCRYPT = 0x02;
        const STORE   = 0x04;
        const FETCH   = 0x08;
    }
}

/// Machine Key Pair — full hybrid key set for a single machine/device.
///
/// Contains Ed25519 + ML-DSA-65 for signing and X25519 + ML-KEM-768 for
/// encryption/key encapsulation. All four key types are always present.
pub struct MachineKeyPair {
    pub(crate) ed25519_signing: ed25519_dalek::SigningKey,
    pub(crate) x25519_secret: x25519_dalek::StaticSecret,
    pub(crate) ml_dsa_signing: ml_dsa::SigningKey<MlDsa65>,
    pub(crate) ml_dsa_verifying: ml_dsa::VerifyingKey<MlDsa65>,
    pub(crate) ml_kem_decap: <MlKem768 as KemCore>::DecapsulationKey,
    pub(crate) ml_kem_encap: <MlKem768 as KemCore>::EncapsulationKey,
    pub(crate) capabilities: MachineKeyCapabilities,
    pub(crate) epoch: u64,
}

impl MachineKeyPair {
    /// Construct from pre-derived seed material.
    pub(crate) fn from_seeds(
        sign_seed: [u8; 32],
        encrypt_seed: [u8; 32],
        pq_sign_seed: [u8; 32],
        pq_encrypt_seed: [u8; 32],
        capabilities: MachineKeyCapabilities,
        epoch: u64,
    ) -> Self {
        let ed25519_signing = ed25519_dalek::SigningKey::from_bytes(&sign_seed);
        let x25519_secret = x25519_dalek::StaticSecret::from(encrypt_seed);

        let ml_dsa_kp = MlDsa65::key_gen_internal(&arr_from_bytes(pq_sign_seed));
        let ml_dsa_verifying = ml_dsa_kp.verifying_key().clone();
        let ml_dsa_signing = ml_dsa_kp.signing_key().clone();

        let (ml_kem_decap, ml_kem_encap) = generate_mlkem_deterministic(&pq_encrypt_seed);

        Self {
            ed25519_signing,
            x25519_secret,
            ml_dsa_signing,
            ml_dsa_verifying,
            ml_kem_decap,
            ml_kem_encap,
            capabilities,
            epoch,
        }
    }

    /// Produce a hybrid signature (Ed25519 + ML-DSA-65) over `msg`.
    pub fn sign(&self, msg: &[u8]) -> HybridSignature {
        use ed25519_dalek::Signer as _;
        use ml_dsa::signature::SignatureEncoding as _;

        let ed_sig = self.ed25519_signing.sign(msg);
        let pq_sig: ml_dsa::Signature<MlDsa65> =
            ml_dsa::signature::Signer::sign(&self.ml_dsa_signing, msg);

        HybridSignature {
            ed25519: ed_sig.to_bytes(),
            ml_dsa: pq_sig.to_bytes().to_vec(),
        }
    }

    /// Extract the corresponding public key.
    pub fn public_key(&self) -> MachinePublicKey {
        use ml_kem::EncodedSizeUser as _;

        let ek_bytes = self.ml_kem_encap.as_bytes();
        let ek_clone =
            <<MlKem768 as KemCore>::EncapsulationKey as ml_kem::EncodedSizeUser>::from_bytes(
                &ek_bytes,
            );

        MachinePublicKey {
            ed25519_verifying: self.ed25519_signing.verifying_key(),
            x25519_public: x25519_dalek::PublicKey::from(&self.x25519_secret),
            ml_dsa_verifying: self.ml_dsa_verifying.clone(),
            ml_kem_encap: ek_clone,
            capabilities: self.capabilities,
            epoch: self.epoch,
        }
    }

    pub fn capabilities(&self) -> MachineKeyCapabilities {
        self.capabilities
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

impl core::fmt::Debug for MachineKeyPair {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MachineKeyPair")
            .field("capabilities", &self.capabilities)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

/// Machine Public Key — all four public key components for a machine.
///
/// Used for signature verification and hybrid key encapsulation.
/// Total size: ~3,200 bytes (32 + 32 + 1,952 + 1,184).
#[derive(Debug)]
pub struct MachinePublicKey {
    pub(crate) ed25519_verifying: ed25519_dalek::VerifyingKey,
    pub(crate) x25519_public: x25519_dalek::PublicKey,
    pub(crate) ml_dsa_verifying: ml_dsa::VerifyingKey<MlDsa65>,
    pub(crate) ml_kem_encap: <MlKem768 as KemCore>::EncapsulationKey,
    pub(crate) capabilities: MachineKeyCapabilities,
    pub(crate) epoch: u64,
}

impl MachinePublicKey {
    /// Verify a hybrid signature: both Ed25519 and ML-DSA-65 must pass.
    pub fn verify(&self, msg: &[u8], sig: &HybridSignature) -> Result<(), CryptoError> {
        use ed25519_dalek::Verifier as _;

        let ed_sig = ed25519_dalek::Signature::from_bytes(&sig.ed25519);
        self.ed25519_verifying
            .verify(msg, &ed_sig)
            .map_err(|_| CryptoError::Ed25519VerifyFailed)?;

        let pq_sig = <ml_dsa::Signature<MlDsa65>>::try_from(sig.ml_dsa.as_slice())
            .map_err(|_| CryptoError::MlDsaVerifyFailed)?;
        ml_dsa::signature::Verifier::verify(&self.ml_dsa_verifying, msg, &pq_sig)
            .map_err(|_| CryptoError::MlDsaVerifyFailed)?;

        Ok(())
    }

    /// Access the raw Ed25519 public key bytes (for DID encoding).
    pub fn ed25519_bytes(&self) -> [u8; 32] {
        self.ed25519_verifying.to_bytes()
    }

    pub fn capabilities(&self) -> MachineKeyCapabilities {
        self.capabilities
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

/// Deterministically generate ML-KEM-768 keys from a 32-byte seed.
fn generate_mlkem_deterministic(
    seed: &[u8; 32],
) -> (
    <MlKem768 as KemCore>::DecapsulationKey,
    <MlKem768 as KemCore>::EncapsulationKey,
) {
    let d = crate::derivation::hkdf_derive_32(seed, b"mlkem768:d")
        .expect("HKDF-SHA256 expand to 32 bytes is infallible");
    let z = crate::derivation::hkdf_derive_32(seed, b"mlkem768:z")
        .expect("HKDF-SHA256 expand to 32 bytes is infallible");

    let d_b32 = mlkem_b32_from_bytes(d);
    let z_b32 = mlkem_b32_from_bytes(z);

    MlKem768::generate_deterministic(&d_b32, &z_b32)
}

/// Construct an ml-kem `B32` from a `[u8; 32]`.
fn mlkem_b32_from_bytes(bytes: [u8; 32]) -> ml_kem::B32 {
    let mut arr = ml_kem::B32::default();
    arr.copy_from_slice(&bytes);
    arr
}
