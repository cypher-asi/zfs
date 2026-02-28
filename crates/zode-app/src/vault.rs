use std::path::Path;

use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};

const ARGON2_SALT_LEN: usize = 16;
const XCHACHA_NONCE_LEN: usize = 24;
const XCHACHA_KEY_LEN: usize = 32;

#[derive(Serialize, Deserialize)]
pub(crate) struct VaultPlaintext {
    pub shares: Vec<String>,
    pub identity_id: [u8; 16],
    pub machine_id: [u8; 16],
    pub epoch: u64,
    pub capabilities: u32,
    pub libp2p_keypair: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct VaultFile {
    pub argon2_salt: [u8; ARGON2_SALT_LEN],
    pub nonce: [u8; XCHACHA_NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

fn derive_key(password: &[u8], salt: &[u8; ARGON2_SALT_LEN]) -> [u8; XCHACHA_KEY_LEN] {
    let argon2 = Argon2::default();
    let mut key = [0u8; XCHACHA_KEY_LEN];
    argon2
        .hash_password_into(password, salt, &mut key)
        .expect("Argon2 output length is valid");
    key
}

pub(crate) fn encrypt_vault(plaintext: &VaultPlaintext, password: &str) -> VaultFile {
    use rand::RngCore;
    let mut salt = [0u8; ARGON2_SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; XCHACHA_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(password.as_bytes(), &salt);
    let cipher = XChaCha20Poly1305::new_from_slice(&key).expect("key length is valid");
    let nonce = XNonce::from_slice(&nonce_bytes);

    let mut cbor_buf = Vec::new();
    ciborium::into_writer(plaintext, &mut cbor_buf).expect("CBOR serialization cannot fail");

    let ciphertext = cipher
        .encrypt(nonce, cbor_buf.as_slice())
        .expect("encryption with random nonce cannot fail");

    VaultFile {
        argon2_salt: salt,
        nonce: nonce_bytes,
        ciphertext,
    }
}

pub(crate) fn decrypt_vault(
    vault: &VaultFile,
    password: &str,
) -> Result<VaultPlaintext, VaultError> {
    let key = derive_key(password.as_bytes(), &vault.argon2_salt);
    let cipher =
        XChaCha20Poly1305::new_from_slice(&key).map_err(|_| VaultError::InvalidKeyLength)?;
    let nonce = XNonce::from_slice(&vault.nonce);

    let cbor_buf = cipher
        .decrypt(nonce, vault.ciphertext.as_slice())
        .map_err(|_| VaultError::DecryptionFailed)?;

    ciborium::de::from_reader(cbor_buf.as_slice()).map_err(|e| VaultError::DeserializeFailed(e.to_string()))
}

pub(crate) fn save_vault(path: &Path, vault: &VaultFile) -> Result<(), VaultError> {
    let mut buf = Vec::new();
    ciborium::into_writer(vault, &mut buf)
        .map_err(|e| VaultError::Io(format!("CBOR encode vault: {e}")))?;
    std::fs::write(path, buf).map_err(|e| VaultError::Io(e.to_string()))
}

pub(crate) fn load_vault(path: &Path) -> Result<VaultFile, VaultError> {
    let data = std::fs::read(path).map_err(|e| VaultError::Io(e.to_string()))?;
    ciborium::de::from_reader(data.as_slice())
        .map_err(|e| VaultError::DeserializeFailed(e.to_string()))
}

#[derive(Debug)]
pub(crate) enum VaultError {
    InvalidKeyLength,
    DecryptionFailed,
    DeserializeFailed(String),
    Io(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::InvalidKeyLength => write!(f, "invalid key length"),
            VaultError::DecryptionFailed => write!(f, "wrong password or corrupted vault"),
            VaultError::DeserializeFailed(e) => write!(f, "vault data corrupted: {e}"),
            VaultError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for VaultError {}
