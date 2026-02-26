use hkdf::Hkdf;
use sha2::Sha256;

use crate::error::CryptoError;
use crate::machine_key::MachineKeyCapabilities;
use crate::neural_key::NeuralKey;
use crate::signing::IdentitySigningKey;

/// Derive an IdentitySigningKey (Ed25519 + ML-DSA-65) from a NeuralKey.
///
/// Domain separation follows zero-id conventions so that the same NeuralKey
/// produces identical Ed25519 keys in both systems.
pub fn derive_identity_signing_key(
    nk: &NeuralKey,
    identity_id: &[u8; 16],
) -> Result<IdentitySigningKey, CryptoError> {
    let ed25519_seed = hkdf_derive_32(nk.as_bytes(), &isk_info(identity_id))?;
    let ml_dsa_seed = hkdf_derive_32(nk.as_bytes(), &isk_pq_info(identity_id))?;

    Ok(IdentitySigningKey::from_seeds(ed25519_seed, ml_dsa_seed))
}

/// Derive a MachineKeyPair (Ed25519 + ML-DSA-65 signing, X25519 + ML-KEM-768 encryption)
/// from a NeuralKey.
///
/// Two-level derivation:
/// 1. Machine seed from NeuralKey with identity_id, machine_id, epoch
/// 2. Individual key seeds from machine seed with algorithm-specific info
pub fn derive_machine_keypair(
    nk: &NeuralKey,
    identity_id: &[u8; 16],
    machine_id: &[u8; 16],
    epoch: u64,
    capabilities: MachineKeyCapabilities,
) -> Result<crate::machine_key::MachineKeyPair, CryptoError> {
    let machine_seed = hkdf_derive_32(
        nk.as_bytes(),
        &machine_seed_info(identity_id, machine_id, epoch),
    )?;

    let sign_seed = hkdf_derive_32(&machine_seed, &machine_sign_info(machine_id))?;
    let encrypt_seed = hkdf_derive_32(&machine_seed, &machine_encrypt_info(machine_id))?;
    let pq_sign_seed = hkdf_derive_32(&machine_seed, &machine_pq_sign_info(machine_id))?;
    let pq_encrypt_seed = hkdf_derive_32(&machine_seed, &machine_pq_encrypt_info(machine_id))?;

    Ok(crate::machine_key::MachineKeyPair::from_seeds(
        sign_seed,
        encrypt_seed,
        pq_sign_seed,
        pq_encrypt_seed,
        capabilities,
        epoch,
    ))
}

/// HKDF-SHA256 extract-then-expand to produce 32 bytes.
pub(crate) fn hkdf_derive_32(ikm: &[u8], info: &[u8]) -> Result<[u8; 32], CryptoError> {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut out = [0u8; 32];
    hk.expand(info, &mut out)
        .map_err(|_| CryptoError::HkdfExpandFailed)?;
    Ok(out)
}

// --- Domain separation info builders ---

fn isk_info(identity_id: &[u8; 16]) -> Vec<u8> {
    let prefix = b"cypher:id:identity:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16);
    info.extend_from_slice(prefix);
    info.extend_from_slice(identity_id);
    info
}

fn isk_pq_info(identity_id: &[u8; 16]) -> Vec<u8> {
    let prefix = b"cypher:id:identity:pq-sign:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16);
    info.extend_from_slice(prefix);
    info.extend_from_slice(identity_id);
    info
}

fn machine_seed_info(identity_id: &[u8; 16], machine_id: &[u8; 16], epoch: u64) -> Vec<u8> {
    let prefix = b"cypher:shared:machine:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16 + 16 + 8);
    info.extend_from_slice(prefix);
    info.extend_from_slice(identity_id);
    info.extend_from_slice(machine_id);
    info.extend_from_slice(&epoch.to_be_bytes());
    info
}

fn machine_sign_info(machine_id: &[u8; 16]) -> Vec<u8> {
    let prefix = b"cypher:shared:machine:sign:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16);
    info.extend_from_slice(prefix);
    info.extend_from_slice(machine_id);
    info
}

fn machine_encrypt_info(machine_id: &[u8; 16]) -> Vec<u8> {
    let prefix = b"cypher:shared:machine:encrypt:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16);
    info.extend_from_slice(prefix);
    info.extend_from_slice(machine_id);
    info
}

fn machine_pq_sign_info(machine_id: &[u8; 16]) -> Vec<u8> {
    let prefix = b"cypher:shared:machine:pq-sign:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16);
    info.extend_from_slice(prefix);
    info.extend_from_slice(machine_id);
    info
}

fn machine_pq_encrypt_info(machine_id: &[u8; 16]) -> Vec<u8> {
    let prefix = b"cypher:shared:machine:pq-encrypt:v1";
    let mut info = Vec::with_capacity(prefix.len() + 16);
    info.extend_from_slice(prefix);
    info.extend_from_slice(machine_id);
    info
}
