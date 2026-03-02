use grid_programs_zephyr::{BlockVote, CertSignature, EpochId, FinalityCertificate, ZoneId};
use std::collections::HashMap;

/// Check if a quorum has been reached for a block hash.
pub fn quorum_reached(vote_count: usize, quorum_threshold: usize) -> bool {
    vote_count >= quorum_threshold
}

/// Collects votes for block proposals and assembles finality certificates.
///
/// Tracks votes per `block_hash` and detects when quorum is reached.
pub struct CertificateBuilder {
    zone_id: ZoneId,
    epoch: EpochId,
    quorum_threshold: usize,
    votes: HashMap<[u8; 32], Vec<BlockVote>>,
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
    ///
    /// The certified `block_hash` becomes the new zone head.
    pub fn add_vote(
        &mut self,
        vote: BlockVote,
        parent_hash: [u8; 32],
    ) -> Option<FinalityCertificate> {
        if vote.zone_id != self.zone_id || vote.epoch != self.epoch {
            return None;
        }

        let entry = self.votes.entry(vote.block_hash).or_default();
        if entry.iter().any(|v| v.voter_id == vote.voter_id) {
            return None;
        }

        entry.push(vote.clone());

        if entry.len() == self.quorum_threshold {
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
                parent_hash,
                block_hash: vote.block_hash,
                signatures,
            })
        } else {
            None
        }
    }

    pub fn vote_count(&self, block_hash: &[u8; 32]) -> usize {
        self.votes.get(block_hash).map_or(0, |v| v.len())
    }

    pub fn has_quorum(&self, block_hash: &[u8; 32]) -> bool {
        quorum_reached(self.vote_count(block_hash), self.quorum_threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vote(block_hash: [u8; 32], voter_byte: u8) -> BlockVote {
        let mut voter_id = [0u8; 32];
        voter_id[0] = voter_byte;
        BlockVote {
            zone_id: 0,
            epoch: 1,
            block_hash,
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
        let parent = [0; 32];

        assert!(builder.add_vote(make_vote(hash, 1), parent).is_none());
        assert!(builder.add_vote(make_vote(hash, 2), parent).is_none());

        let cert = builder.add_vote(make_vote(hash, 3), parent);
        assert!(cert.is_some());

        let cert = cert.unwrap();
        assert_eq!(cert.zone_id, 0);
        assert_eq!(cert.epoch, 1);
        assert_eq!(cert.block_hash, hash);
        assert_eq!(cert.signatures.len(), 3);
        assert_eq!(cert.parent_hash, parent);
    }

    #[test]
    fn duplicate_voter_ignored() {
        let mut builder = CertificateBuilder::new(0, 1, 3);
        let hash = [0xBB; 32];
        let parent = [0; 32];

        builder.add_vote(make_vote(hash, 1), parent);
        builder.add_vote(make_vote(hash, 1), parent);
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
    fn has_quorum_reflects_state() {
        let mut builder = CertificateBuilder::new(0, 1, 2);
        let hash = [0xDD; 32];
        let parent = [0; 32];

        assert!(!builder.has_quorum(&hash));
        builder.add_vote(make_vote(hash, 1), parent);
        assert!(!builder.has_quorum(&hash));
        builder.add_vote(make_vote(hash, 2), parent);
        assert!(builder.has_quorum(&hash));
    }
}
