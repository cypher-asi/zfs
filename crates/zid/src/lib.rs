#![forbid(unsafe_code)]

pub(crate) mod derivation;
pub mod did;
pub mod encapsulation;
pub mod error;
pub mod machine_key;
pub(crate) mod neural_key;
pub mod shamir;
pub mod shares_api;
pub mod signing;

pub use did::{did_key_to_ed25519, ed25519_to_did_key, verify_did_ed25519};
pub use encapsulation::{EncapBundle, SharedSecret};
pub use error::CryptoError;
pub use machine_key::{MachineKeyCapabilities, MachineKeyPair, MachinePublicKey};
pub use shamir::ShamirShare;
pub use shares_api::{
    derive_machine_keypair_from_shares, generate_identity, sign_with_shares, verify_shares,
    IdentityBundle, IdentityInfo,
};
pub use signing::{HybridSignature, IdentitySigningKey, IdentityVerifyingKey};

/// Test-only helpers for deterministic key derivation from raw seeds.
/// External crates add `zid = { ..., features = ["testkit"] }` in `[dev-dependencies]`.
#[cfg(feature = "testkit")]
pub mod testkit {
    use crate::error::CryptoError;
    use crate::machine_key::{MachineKeyCapabilities, MachineKeyPair};
    use crate::neural_key::NeuralKey;
    use crate::signing::IdentitySigningKey;

    pub fn derive_identity_signing_key_from_seed(
        seed: [u8; 32],
        identity_id: &[u8; 16],
    ) -> Result<IdentitySigningKey, CryptoError> {
        let nk = NeuralKey::from_bytes(seed);
        crate::derivation::derive_identity_signing_key(&nk, identity_id)
    }

    pub fn derive_machine_keypair_from_seed(
        seed: [u8; 32],
        identity_id: &[u8; 16],
        machine_id: &[u8; 16],
        epoch: u64,
        capabilities: MachineKeyCapabilities,
    ) -> Result<MachineKeyPair, CryptoError> {
        let nk = NeuralKey::from_bytes(seed);
        crate::derivation::derive_machine_keypair(&nk, identity_id, machine_id, epoch, capabilities)
    }
}
