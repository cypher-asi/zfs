use std::sync::Arc;

type SigningFn = Arc<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>;

/// Read-only view of the Zode's node identity.
///
/// Provides the node's peer ID and Ed25519 signing capability so that
/// services can identify themselves and sign data without managing
/// their own key material.
pub struct NodeIdentity {
    zode_id: String,
    public_key: Vec<u8>,
    signing_fn: SigningFn,
}

impl NodeIdentity {
    pub fn new(zode_id: String, public_key: Vec<u8>, signing_fn: SigningFn) -> Self {
        Self {
            zode_id,
            public_key,
            signing_fn,
        }
    }

    /// The node's formatted ZodeId (e.g. `Zx12D3KooW...`).
    pub fn zode_id(&self) -> &str {
        &self.zode_id
    }

    /// The raw Ed25519 public key bytes.
    pub fn public_key(&self) -> &[u8] {
        &self.public_key
    }

    /// Sign arbitrary data with the node's Ed25519 private key.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        (self.signing_fn)(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_exposes_zode_id_and_public_key() {
        let identity = NodeIdentity::new(
            "ZxTestPeer".into(),
            vec![1, 2, 3],
            Arc::new(|data: &[u8]| data.to_vec()),
        );

        assert_eq!(identity.zode_id(), "ZxTestPeer");
        assert_eq!(identity.public_key(), &[1, 2, 3]);
    }

    #[test]
    fn sign_delegates_to_signing_fn() {
        let identity = NodeIdentity::new(
            "ZxTestPeer".into(),
            vec![],
            Arc::new(|data: &[u8]| {
                let mut sig = data.to_vec();
                sig.push(0xFF);
                sig
            }),
        );

        let sig = identity.sign(b"hello");
        assert_eq!(sig, b"hello\xFF");
    }
}
