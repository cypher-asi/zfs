use grid_programs_zephyr::{BatchVote, CertSignature, EpochId, FinalityCertificate, ZoneId};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// Check if a quorum has been reached for a batch hash.
pub fn quorum_reached(vote_count: usize, quorum_threshold: usize) -> bool {
    vote_count >= quorum_threshold
}

/// Collects votes for batch proposals and assembles finality certificates.
///
/// Tracks votes per `batch_hash` and detects when quorum is reached.
pub struct CertificateBuilder {
    zone_id: ZoneId,
    epoch: EpochId,
    quorum_threshold: usize,
    votes: HashMap<[u8; 32], Vec<BatchVote>>,
}

impl CertificateBuilder {
    pub fn new(zone_id: ZoneId, epoch: EpochId, quorum_threshold: usize) -> Self {
        Self {
            zone_id,
            epoch,
            quorum_threshold,
            votes: HashMap::new(),
        }
    }

    /// Add a vote. Returns `Some(FinalityCertificate)` if quorum is newly reached.
    pub fn add_vote(
        &mut self,
        vote: BatchVote,
        prev_zone_head: [u8; 32],
    ) -> Option<FinalityCertificate> {
        if vote.zone_id != self.zone_id || vote.epoch != self.epoch {
            return None;
        }

        let entry = self.votes.entry(vote.batch_hash).or_default();
        if entry.iter().any(|v| v.voter_id == vote.voter_id) {
            return None;
        }

        entry.push(vote.clone());

        if entry.len() == self.quorum_threshold {
            let new_zone_head = compute_new_zone_head(&vote.batch_hash, &prev_zone_head);
            let signatures = entry
                .iter()
                .map(|v| CertSignature {
                    validator_id: v.voter_id,
                    signature: v.signature.clone(),
                })
                .collect();

            Some(FinalityCertificate {
                zone_id: self.zone_id,
                epoch: self.epoch,
                prev_zone_head,
                new_zone_head,
                batch_hash: vote.batch_hash,
                signatures,
            })
        } else {
            None
        }
    }

    pub fn vote_count(&self, batch_hash: &[u8; 32]) -> usize {
        self.votes.get(batch_hash).map_or(0, |v| v.len())
    }

    pub fn has_quorum(&self, batch_hash: &[u8; 32]) -> bool {
        quorum_reached(self.vote_count(batch_hash), self.quorum_threshold)
    }
}

/// `new_zone_head = SHA-256(batch_hash || prev_zone_head)`
pub fn compute_new_zone_head(batch_hash: &[u8; 32], prev_zone_head: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(batch_hash);
    hasher.update(prev_zone_head);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vote(batch_hash: [u8; 32], voter_byte: u8) -> BatchVote {
        let mut voter_id = [0u8; 32];
        voter_id[0] = voter_byte;
        BatchVote {
            zone_id: 0,
            epoch: 1,
            batch_hash,
            voter_id,
            signature: vec![voter_byte],
        }
    }

    #[test]
    fn quorum_check() {
        assert!(!quorum_reached(3, 4));
        assert!(quorum_reached(4, 4));
        assert!(quorum_reached(5, 4));
    }

    #[test]
    fn certificate_built_at_quorum() {
        let mut builder = CertificateBuilder::new(0, 1, 3);
        let hash = [0xAA; 32];
        let prev = [0; 32];

        assert!(builder.add_vote(make_vote(hash, 1), prev).is_none());
        assert!(builder.add_vote(make_vote(hash, 2), prev).is_none());

        let cert = builder.add_vote(make_vote(hash, 3), prev);
        assert!(cert.is_some());

        let cert = cert.unwrap();
        assert_eq!(cert.zone_id, 0);
        assert_eq!(cert.epoch, 1);
        assert_eq!(cert.batch_hash, hash);
        assert_eq!(cert.signatures.len(), 3);
        assert_eq!(cert.prev_zone_head, prev);
        assert_eq!(cert.new_zone_head, compute_new_zone_head(&hash, &prev));
    }

    #[test]
    fn duplicate_voter_ignored() {
        let mut builder = CertificateBuilder::new(0, 1, 3);
        let hash = [0xBB; 32];
        let prev = [0; 32];

        builder.add_vote(make_vote(hash, 1), prev);
        builder.add_vote(make_vote(hash, 1), prev);
        assert_eq!(builder.vote_count(&hash), 1);
    }

    #[test]
    fn wrong_zone_ignored() {
        let mut builder = CertificateBuilder::new(0, 1, 2);
        let hash = [0xCC; 32];

        let mut vote = make_vote(hash, 1);
        vote.zone_id = 99;
        assert!(builder.add_vote(vote, [0; 32]).is_none());
        assert_eq!(builder.vote_count(&hash), 0);
    }

    #[test]
    fn new_zone_head_deterministic() {
        let batch = [1u8; 32];
        let prev = [2u8; 32];
        let h1 = compute_new_zone_head(&batch, &prev);
        let h2 = compute_new_zone_head(&batch, &prev);
        assert_eq!(h1, h2);
    }

    #[test]
    fn has_quorum_reflects_state() {
        let mut builder = CertificateBuilder::new(0, 1, 2);
        let hash = [0xDD; 32];
        let prev = [0; 32];

        assert!(!builder.has_quorum(&hash));
        builder.add_vote(make_vote(hash, 1), prev);
        assert!(!builder.has_quorum(&hash));
        builder.add_vote(make_vote(hash, 2), prev);
        assert!(builder.has_quorum(&hash));
    }
}
