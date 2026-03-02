use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305};
use grid_core::{ProgramId, SectorId};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zid::{ed25519_to_did_key, MachineKeyPair, MachinePublicKey};

use crate::{CryptoError, SectorKey};

const WRAP_INFO_PREFIX: &[u8] = b"grid:sector-key-wrap:v1";
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;

/// Sender's static X25519 public key (32 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SenderX25519Public(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl AsRef<[u8]> for SenderX25519Public {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// ML-KEM-768 ciphertext (1,088 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MlkemCiphertext(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl AsRef<[u8]> for MlkemCiphertext {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Wrapped sector key: `nonce (24) || encrypted_key (32) || tag (16)` = 72 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WrappedKey(#[serde(with = "serde_bytes")] pub Vec<u8>);

impl AsRef<[u8]> for WrappedKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// A single recipient entry within a key envelope.
///
/// Contains the hybrid-wrapped sector key for one recipient,
/// identified by their `did:key`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyEnvelopeEntry {
    pub recipient_did: String,
    pub sender_x25519_public: SenderX25519Public,
    pub mlkem_ciphertext: MlkemCiphertext,
    pub wrapped_key: WrappedKey,
}

/// Derive the context-bound wrap key from a shared secret and sector context.
fn derive_wrap_key(
    shared_secret: &[u8; 32],
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<[u8; 32], CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, shared_secret);

    let mut info = Vec::with_capacity(WRAP_INFO_PREFIX.len() + 32 + sector_id.as_bytes().len());
    info.extend_from_slice(WRAP_INFO_PREFIX);
    info.extend_from_slice(program_id.as_bytes());
    info.extend_from_slice(sector_id.as_bytes());

    let mut wrap_key = [0u8; 32];
    hk.expand(&info, &mut wrap_key)
        .map_err(|_| CryptoError::HkdfExpandFailed)?;
    Ok(wrap_key)
}

/// Wrap a SectorKey for a recipient using hybrid key agreement.
///
/// 1. Performs hybrid key agreement (`zid` X25519 + ML-KEM-768 encapsulation)
///    between sender and recipient.
/// 2. Derives a context-bound wrap key via HKDF with `program_id || sector_id`.
/// 3. Encrypts the SectorKey with XChaCha20-Poly1305.
pub fn wrap_sector_key(
    sector_key: &SectorKey,
    sender: &MachineKeyPair,
    recipient_public: &MachinePublicKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<KeyEnvelopeEntry, CryptoError> {
    let (shared_secret, bundle) = recipient_public.encapsulate(sender)?;
    let wrap_key = derive_wrap_key(shared_secret.as_bytes(), program_id, sector_id)?;

    let cipher = XChaCha20Poly1305::new((&wrap_key).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

    let encrypted = cipher
        .encrypt(&nonce, sector_key.as_bytes().as_ref())
        .map_err(|_| CryptoError::EncryptionFailed)?;

    // nonce (24) || encrypted sector key (32) || tag (16) = 72 bytes
    let mut wrapped_bytes = Vec::with_capacity(NONCE_LEN + encrypted.len());
    wrapped_bytes.extend_from_slice(&nonce);
    wrapped_bytes.extend_from_slice(&encrypted);

    let recipient_did = ed25519_to_did_key(&recipient_public.ed25519_bytes());

    Ok(KeyEnvelopeEntry {
        recipient_did,
        sender_x25519_public: SenderX25519Public(bundle.x25519_public.to_vec()),
        mlkem_ciphertext: MlkemCiphertext(bundle.mlkem_ciphertext.clone()),
        wrapped_key: WrappedKey(wrapped_bytes),
    })
}

/// Unwrap a SectorKey from a [`KeyEnvelopeEntry`].
///
/// Reverses [`wrap_sector_key`]: decapsulates the shared secret, derives the
/// wrap key, then decrypts the SectorKey.
pub fn unwrap_sector_key(
    entry: &KeyEnvelopeEntry,
    recipient: &MachineKeyPair,
    sender_public: &MachinePublicKey,
    program_id: &ProgramId,
    sector_id: &SectorId,
) -> Result<SectorKey, CryptoError> {
    let bundle = zid::EncapBundle {
        x25519_public: entry
            .sender_x25519_public
            .as_ref()
            .try_into()
            .map_err(|_| zid::CryptoError::InvalidKeyLength {
                expected: 32,
                got: entry.sender_x25519_public.0.len(),
            })?,
        mlkem_ciphertext: entry.mlkem_ciphertext.0.clone(),
    };

    let shared_secret = recipient.decapsulate(&bundle, sender_public)?;
    let wrap_key = derive_wrap_key(shared_secret.as_bytes(), program_id, sector_id)?;

    let min_len = NONCE_LEN + TAG_LEN;
    if entry.wrapped_key.0.len() < min_len {
        return Err(CryptoError::CiphertextTooShort {
            len: entry.wrapped_key.0.len(),
            min: min_len,
        });
    }

    let (nonce_bytes, ct) = entry.wrapped_key.0.split_at(NONCE_LEN);
    let nonce = chacha20poly1305::XNonce::from_slice(nonce_bytes);
    let cipher = XChaCha20Poly1305::new((&wrap_key).into());

    let plaintext = cipher
        .decrypt(nonce, ct)
        .map_err(|_| CryptoError::DecryptionFailed)?;

    let key_bytes: [u8; 32] =
        plaintext
            .try_into()
            .map_err(|v: Vec<u8>| CryptoError::CiphertextTooShort {
                len: v.len(),
                min: 32,
            })?;
    Ok(SectorKey::from_bytes(key_bytes))
}
