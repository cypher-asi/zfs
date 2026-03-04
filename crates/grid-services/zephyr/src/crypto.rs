use ed25519_dalek::{Signature, VerifyingKey};
use grid_service::NodeIdentity;

/// Verify an Ed25519 signature against a 32-byte public key.
pub fn verify_ed25519(pubkey: &[u8; 32], message: &[u8], signature: &[u8]) -> bool {
    let Ok(vk) = VerifyingKey::from_bytes(pubkey) else {
        return false;
    };
    let Ok(sig) = Signature::try_from(signature) else {
        return false;
    };
    vk.verify_strict(message, &sig).is_ok()
}

/// Sign data using the node's Ed25519 identity key.
pub fn sign_ed25519(identity: &NodeIdentity, data: &[u8]) -> Vec<u8> {
    identity.sign(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_rejects_wrong_signature() {
        let pubkey = [0u8; 32];
        let message = b"hello";
        let bad_sig = [0u8; 64];
        assert!(!verify_ed25519(&pubkey, message, &bad_sig));
    }

    #[test]
    fn verify_rejects_truncated_signature() {
        let pubkey = [1u8; 32];
        assert!(!verify_ed25519(&pubkey, b"msg", &[0u8; 32]));
    }
}
