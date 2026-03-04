use grid_programs_zephyr::FinalityCertificate;
use tracing::debug;

use super::engine::ZoneConsensus;

/// Bounded buffer for out-of-order certificates waiting for their parent
/// to arrive. Deduplicates by `block_hash` and purges stale entries.
pub struct PendingCertBuffer {
    certs: Vec<FinalityCertificate>,
    max_size: usize,
}

impl PendingCertBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            certs: Vec::new(),
            max_size,
        }
    }

    /// Insert a certificate. Returns `false` if the buffer is full or
    /// the cert is a duplicate.
    pub fn push(&mut self, cert: FinalityCertificate) -> bool {
        if self.certs.iter().any(|c| c.block_hash == cert.block_hash) {
            return false;
        }
        if self.certs.len() >= self.max_size {
            return false;
        }
        self.certs.push(cert);
        true
    }

    /// Sort by height and repeatedly apply any certificate whose parent matches
    /// the engine's current head. Returns the applied certs so callers can run
    /// side-effects (zone-head store, mempool cleanup).
    pub fn drain_applicable(
        &mut self,
        eng: &mut ZoneConsensus,
    ) -> Vec<FinalityCertificate> {
        self.certs.sort_by_key(|c| c.height);
        let mut applied = Vec::new();
        let mut progress = true;
        while progress && !self.certs.is_empty() {
            progress = false;
            let mut keep = Vec::new();
            for pc in self.certs.drain(..) {
                if pc.block_hash == *eng.parent_hash() || pc.height < eng.height() {
                    debug!(
                        zone_id = eng.zone_id(),
                        cert_block = %hex::encode(&pc.block_hash[..8]),
                        "purging stale buffered certificate"
                    );
                    continue;
                }
                if eng.apply_certificate(&pc) {
                    let _ = eng.take_fork_recovery_used();
                    applied.push(pc);
                    progress = true;
                } else {
                    keep.push(pc);
                }
            }
            self.certs = keep;
        }
        applied
    }

    /// Drop certs from epochs older than `min_epoch - 1`.
    pub fn retain_epoch(&mut self, min_epoch: u64) {
        self.certs.retain(|c| c.epoch + 1 >= min_epoch);
    }

    /// Purge oldest entries when above 75% capacity.
    pub fn purge_overflow(&mut self) {
        let threshold = self.max_size * 3 / 4;
        if self.certs.len() > threshold {
            let drop_count = self.certs.len() - threshold;
            self.certs.drain(..drop_count);
        }
    }

    pub fn len(&self) -> usize {
        self.certs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.certs.is_empty()
    }

    pub fn clear(&mut self) {
        self.certs.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = &FinalityCertificate> {
        self.certs.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ZephyrConfig;
    use crate::consensus::leader::leader_for_round;
    use ed25519_dalek::{Signer, SigningKey};
    use grid_programs_zephyr::{CertSignature, FinalityCertificate, ValidatorInfo};

    fn make_keys(n: usize) -> Vec<SigningKey> {
        (0..n)
            .map(|i| {
                let mut seed = [0u8; 32];
                seed[0] = i as u8;
                SigningKey::from_bytes(&seed)
            })
            .collect()
    }

    fn make_committee_from_keys(keys: &[SigningKey]) -> Vec<ValidatorInfo> {
        keys.iter()
            .enumerate()
            .map(|(i, sk)| {
                let pk = sk.verifying_key().to_bytes();
                ValidatorInfo {
                    validator_id: pk,
                    pubkey: pk,
                    p2p_endpoint: format!("/ip4/127.0.0.1/tcp/{}", 4000 + i),
                }
            })
            .collect()
    }

    fn test_config() -> ZephyrConfig {
        ZephyrConfig {
            total_zones: 4,
            committee_size: 3,
            quorum_threshold: 2,
            max_block_size: 64,
            ..ZephyrConfig::default()
        }
    }

    fn make_unsigned_cert(
        zone: u32,
        epoch: u64,
        height: u64,
        parent: [u8; 32],
        block: [u8; 32],
    ) -> FinalityCertificate {
        FinalityCertificate {
            zone_id: zone,
            epoch,
            height,
            block_hash: block,
            parent_hash: parent,
            signatures: vec![],
        }
    }

    fn make_signed_cert(
        zone: u32,
        epoch: u64,
        height: u64,
        parent: [u8; 32],
        block: [u8; 32],
        keys: &[SigningKey],
        signer_indices: &[usize],
    ) -> FinalityCertificate {
        FinalityCertificate {
            zone_id: zone,
            epoch,
            height,
            block_hash: block,
            parent_hash: parent,
            signatures: signer_indices
                .iter()
                .map(|&i| CertSignature {
                    validator_id: keys[i].verifying_key().to_bytes(),
                    signature: keys[i].sign(&block).to_bytes().to_vec(),
                })
                .collect(),
        }
    }

    #[test]
    fn push_rejects_duplicate() {
        let mut buf = PendingCertBuffer::new(10);
        let cert = make_unsigned_cert(0, 0, 0, [0xAA; 32], [0xBB; 32]);
        assert!(buf.push(cert.clone()));
        assert!(!buf.push(cert));
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn push_rejects_when_full() {
        let mut buf = PendingCertBuffer::new(2);
        assert!(buf.push(make_unsigned_cert(0, 0, 0, [1; 32], [2; 32])));
        assert!(buf.push(make_unsigned_cert(0, 0, 1, [2; 32], [3; 32])));
        assert!(!buf.push(make_unsigned_cert(0, 0, 2, [3; 32], [4; 32])));
    }

    #[test]
    fn drain_applicable_chains_certs() {
        let keys = make_keys(3);
        let committee = make_committee_from_keys(&keys);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut eng =
            ZoneConsensus::new(0, 0, committee, leader_id, [0xAA; 32], test_config(), 0);

        let mut buf = PendingCertBuffer::new(10);
        buf.push(make_signed_cert(
            0,
            0,
            1,
            [0xBB; 32],
            [0xCC; 32],
            &keys,
            &[0, 1],
        ));
        buf.push(make_signed_cert(
            0,
            0,
            0,
            [0xAA; 32],
            [0xBB; 32],
            &keys,
            &[0, 1],
        ));

        let applied = buf.drain_applicable(&mut eng);
        assert_eq!(applied.len(), 2);
        assert_eq!(eng.height(), 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn retain_epoch_drops_old() {
        let mut buf = PendingCertBuffer::new(10);
        buf.push(make_unsigned_cert(0, 0, 0, [1; 32], [2; 32]));
        buf.push(make_unsigned_cert(0, 4, 0, [3; 32], [4; 32]));
        buf.push(make_unsigned_cert(0, 5, 0, [5; 32], [6; 32]));
        buf.retain_epoch(5);
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn purge_overflow_trims() {
        let mut buf = PendingCertBuffer::new(8);
        for i in 0..7u8 {
            buf.push(make_unsigned_cert(0, 0, i as u64, [i; 32], [i + 100; 32]));
        }
        assert_eq!(buf.len(), 7);
        buf.purge_overflow();
        assert_eq!(buf.len(), 6);
    }
}
