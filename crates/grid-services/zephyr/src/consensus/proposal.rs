use grid_programs_zephyr::{BatchProposal, EpochId, Nullifier, SpendTransaction, ZoneId};
use sha2::{Digest, Sha256};

/// Construct a batch proposal from a set of verified spends.
///
/// The batch hash binds the proposal to the zone, epoch, previous head,
/// and the ordered set of nullifiers. This is signed by the proposer.
pub fn build_batch_proposal(
    zone_id: ZoneId,
    epoch: EpochId,
    prev_zone_head: [u8; 32],
    spends: Vec<SpendTransaction>,
    proposer_id: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
) -> BatchProposal {
    let nullifiers: Vec<Nullifier> = spends.iter().map(|s| s.nullifier.clone()).collect();
    let batch_hash = compute_batch_hash(zone_id, epoch, &prev_zone_head, &nullifiers);
    let proposer_sig = sign_fn(&batch_hash);

    BatchProposal {
        zone_id,
        epoch,
        prev_zone_head,
        nullifiers,
        spends,
        batch_hash,
        proposer_id,
        proposer_sig,
    }
}

/// `batch_hash = SHA-256(zone_id || epoch || prev_zone_head || nullifier_root)`
///
/// The nullifier root is `SHA-256(n_0 || n_1 || ... || n_k)` — a flat
/// hash for simplicity in MVP (Merkle tree can replace this later).
pub fn compute_batch_hash(
    zone_id: ZoneId,
    epoch: EpochId,
    prev_zone_head: &[u8; 32],
    nullifiers: &[Nullifier],
) -> [u8; 32] {
    let nullifier_root = {
        let mut hasher = Sha256::new();
        for n in nullifiers {
            hasher.update(n.as_ref());
        }
        hasher.finalize()
    };

    let mut hasher = Sha256::new();
    hasher.update(zone_id.to_be_bytes());
    hasher.update(epoch.to_be_bytes());
    hasher.update(prev_zone_head);
    hasher.update(nullifier_root);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_programs_zephyr::NoteCommitment;

    fn dummy_spend(nullifier_byte: u8) -> SpendTransaction {
        SpendTransaction {
            input_commitment: NoteCommitment([0; 32]),
            nullifier: Nullifier([nullifier_byte; 32]),
            outputs: vec![],
            proof: vec![],
            public_signals: vec![],
        }
    }

    #[test]
    fn batch_hash_is_deterministic() {
        let nullifiers = vec![Nullifier([1; 32]), Nullifier([2; 32])];
        let h1 = compute_batch_hash(0, 1, &[0; 32], &nullifiers);
        let h2 = compute_batch_hash(0, 1, &[0; 32], &nullifiers);
        assert_eq!(h1, h2);
    }

    #[test]
    fn batch_hash_changes_with_zone() {
        let nullifiers = vec![Nullifier([1; 32])];
        let h1 = compute_batch_hash(0, 1, &[0; 32], &nullifiers);
        let h2 = compute_batch_hash(1, 1, &[0; 32], &nullifiers);
        assert_ne!(h1, h2);
    }

    #[test]
    fn build_proposal_includes_all_nullifiers() {
        let spends = vec![dummy_spend(1), dummy_spend(2)];
        let proposal =
            build_batch_proposal(0, 1, [0; 32], spends, [0xAA; 32], |hash| hash.to_vec());
        assert_eq!(proposal.nullifiers.len(), 2);
        assert_eq!(proposal.spends.len(), 2);
        assert_eq!(proposal.proposer_id, [0xAA; 32]);
    }

    #[test]
    fn build_proposal_signs_batch_hash() {
        let spends = vec![dummy_spend(1)];
        let proposal =
            build_batch_proposal(0, 1, [0; 32], spends, [0xBB; 32], |hash| hash.to_vec());
        assert_eq!(proposal.proposer_sig, proposal.batch_hash.to_vec());
    }
}
