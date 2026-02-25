use rand_core::{CryptoRng, RngCore};
use zeroize::Zeroize;

use crate::derivation::{derive_identity_signing_key, derive_machine_keypair};
use crate::did::ed25519_to_did_key;
use crate::error::CryptoError;
use crate::machine_key::{MachineKeyCapabilities, MachineKeyPair};
use crate::neural_key::NeuralKey;
use crate::shamir::{self, ShamirShare};
use crate::signing::{HybridSignature, IdentityVerifyingKey};

/// Everything returned after generating a new identity.
pub struct IdentityBundle {
    pub shares: Vec<ShamirShare>,
    pub threshold: usize,
    pub verifying_key: IdentityVerifyingKey,
    pub did: String,
}

/// Public identity info recovered from shares (no secret material).
pub struct IdentityInfo {
    pub verifying_key: IdentityVerifyingKey,
    pub did: String,
}

/// Generate a new identity. Returns shares + public identity info.
/// The NeuralKey is created, split, and immediately zeroized.
pub fn generate_identity(
    threshold: usize,
    total: usize,
    identity_id: &[u8; 16],
    rng: &mut (impl RngCore + CryptoRng),
) -> Result<IdentityBundle, CryptoError> {
    let nk = NeuralKey::generate(rng);
    let mut secret = nk.to_bytes();

    let shares = shamir::split(&secret, total, threshold, rng)?;

    let isk = derive_identity_signing_key(&nk, identity_id)?;
    let vk = isk.verifying_key();
    let did = ed25519_to_did_key(&isk.ed25519_public_bytes());

    secret.zeroize();
    drop(nk);

    Ok(IdentityBundle {
        shares,
        threshold,
        verifying_key: vk,
        did,
    })
}

/// Verify shares are valid by reconstructing and checking. Returns public identity info.
pub fn verify_shares(
    shares: &[ShamirShare],
    identity_id: &[u8; 16],
) -> Result<IdentityInfo, CryptoError> {
    let mut secret = shamir::combine(shares)?;
    let nk = NeuralKey::from_bytes(secret);
    secret.zeroize();

    let isk = derive_identity_signing_key(&nk, identity_id)?;
    let vk = isk.verifying_key();
    let did = ed25519_to_did_key(&isk.ed25519_public_bytes());

    drop(nk);

    Ok(IdentityInfo {
        verifying_key: vk,
        did,
    })
}

/// Sign a message using threshold shares. Ephemeral reconstruction + zeroize.
pub fn sign_with_shares(
    shares: &[ShamirShare],
    identity_id: &[u8; 16],
    msg: &[u8],
) -> Result<HybridSignature, CryptoError> {
    let mut secret = shamir::combine(shares)?;
    let nk = NeuralKey::from_bytes(secret);
    secret.zeroize();

    let isk = derive_identity_signing_key(&nk, identity_id)?;
    let sig = isk.sign(msg);

    drop(nk);
    Ok(sig)
}

/// Derive a machine keypair using threshold shares. Ephemeral reconstruction + zeroize.
pub fn derive_machine_keypair_from_shares(
    shares: &[ShamirShare],
    identity_id: &[u8; 16],
    machine_id: &[u8; 16],
    epoch: u64,
    capabilities: MachineKeyCapabilities,
) -> Result<MachineKeyPair, CryptoError> {
    let mut secret = shamir::combine(shares)?;
    let nk = NeuralKey::from_bytes(secret);
    secret.zeroize();

    let kp = derive_machine_keypair(&nk, identity_id, machine_id, epoch, capabilities)?;

    drop(nk);
    Ok(kp)
}
