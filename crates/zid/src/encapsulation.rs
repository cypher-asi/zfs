use ml_kem::kem::{Decapsulate, Encapsulate};
use ml_kem::MlKem768;
use rand_core::OsRng;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::derivation::hkdf_derive_32;
use crate::error::CryptoError;
use crate::machine_key::{MachineKeyPair, MachinePublicKey};

/// ML-KEM-768 ciphertext length in bytes.
const ML_KEM_768_CT_LEN: usize = 1_088;

/// Combined shared secret from hybrid X25519 + ML-KEM-768 key agreement.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SharedSecret(pub(crate) [u8; 32]);

impl SharedSecret {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl core::fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("SharedSecret").field(&"[REDACTED]").finish()
    }
}

/// Bundle produced by hybrid encapsulation: sender's X25519 public key
/// and the ML-KEM-768 ciphertext.
#[derive(Debug, Clone)]
pub struct EncapBundle {
    pub x25519_public: [u8; 32],
    pub mlkem_ciphertext: Vec<u8>,
}

impl EncapBundle {
    /// Serialize to bytes: x25519_public (32) || mlkem_ciphertext (1088).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + self.mlkem_ciphertext.len());
        out.extend_from_slice(&self.x25519_public);
        out.extend_from_slice(&self.mlkem_ciphertext);
        out
    }

    /// Deserialize from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < 32 + ML_KEM_768_CT_LEN {
            return Err(CryptoError::InvalidCiphertextLength {
                expected: 32 + ML_KEM_768_CT_LEN,
                got: bytes.len(),
            });
        }
        let mut x25519_public = [0u8; 32];
        x25519_public.copy_from_slice(&bytes[..32]);
        let mlkem_ciphertext = bytes[32..32 + ML_KEM_768_CT_LEN].to_vec();
        Ok(Self {
            x25519_public,
            mlkem_ciphertext,
        })
    }
}

impl MachinePublicKey {
    /// Hybrid encapsulate to this public key using sender's key pair.
    ///
    /// Combines X25519 static DH and ML-KEM-768 encapsulation, then
    /// derives a shared secret via HKDF over both shared secrets.
    pub fn encapsulate(
        &self,
        sender: &MachineKeyPair,
    ) -> Result<(SharedSecret, EncapBundle), CryptoError> {
        let x25519_ss = sender.x25519_secret.diffie_hellman(&self.x25519_public);
        let x25519_bytes = x25519_ss.as_bytes();
        if x25519_bytes.iter().all(|&b| b == 0) {
            return Err(CryptoError::X25519ZeroSharedSecret);
        }

        let (mlkem_ct, mlkem_ss): (ml_kem::Ciphertext<MlKem768>, ml_kem::SharedKey<MlKem768>) =
            self.ml_kem_encap
                .encapsulate(&mut OsRng)
                .expect("ML-KEM-768 encapsulation is infallible");

        let mut mlkem_bytes = [0u8; 32];
        mlkem_bytes.copy_from_slice(mlkem_ss.as_ref());
        let combined = combine_shared_secrets(x25519_bytes, &mlkem_bytes)?;

        let sender_x25519_pk = x25519_dalek::PublicKey::from(&sender.x25519_secret);
        let bundle = EncapBundle {
            x25519_public: sender_x25519_pk.to_bytes(),
            mlkem_ciphertext: AsRef::<[u8]>::as_ref(&mlkem_ct).to_vec(),
        };

        Ok((SharedSecret(combined), bundle))
    }
}

impl MachineKeyPair {
    /// Hybrid decapsulate using this key pair (recipient) and the sender's public key.
    pub fn decapsulate(
        &self,
        bundle: &EncapBundle,
        sender_public: &MachinePublicKey,
    ) -> Result<SharedSecret, CryptoError> {
        let x25519_ss = self
            .x25519_secret
            .diffie_hellman(&sender_public.x25519_public);
        let x25519_bytes = x25519_ss.as_bytes();
        if x25519_bytes.iter().all(|&b| b == 0) {
            return Err(CryptoError::X25519ZeroSharedSecret);
        }

        if bundle.mlkem_ciphertext.len() != ML_KEM_768_CT_LEN {
            return Err(CryptoError::InvalidCiphertextLength {
                expected: ML_KEM_768_CT_LEN,
                got: bundle.mlkem_ciphertext.len(),
            });
        }
        let ct: ml_kem::Ciphertext<MlKem768> = {
            let mut arr = <ml_kem::Ciphertext<MlKem768>>::default();
            arr.copy_from_slice(&bundle.mlkem_ciphertext);
            arr
        };

        let mlkem_ss: ml_kem::SharedKey<MlKem768> = self
            .ml_kem_decap
            .decapsulate(&ct)
            .map_err(|_| CryptoError::MlKemDecapFailed)?;

        let mut mlkem_bytes = [0u8; 32];
        mlkem_bytes.copy_from_slice(mlkem_ss.as_ref());
        let combined = combine_shared_secrets(x25519_bytes, &mlkem_bytes)?;
        Ok(SharedSecret(combined))
    }
}

/// HKDF-combine two shared secrets into a single 32-byte key.
fn combine_shared_secrets(x25519_ss: &[u8], mlkem_ss: &[u8]) -> Result<[u8; 32], CryptoError> {
    let mut ikm = Vec::with_capacity(x25519_ss.len() + mlkem_ss.len());
    ikm.extend_from_slice(x25519_ss);
    ikm.extend_from_slice(mlkem_ss);

    hkdf_derive_32(&ikm, b"zid:encap:v1")
}
