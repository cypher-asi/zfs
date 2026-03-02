use grid_programs_zephyr::{Block, BlockHeader, EpochId, Nullifier, SpendTransaction, ZoneId};
use sha2::{Digest, Sha256};

/// Parameters for building a new block.
pub struct BlockParams {
    pub zone_id: ZoneId,
    pub epoch: EpochId,
    pub height: u64,
    pub parent_hash: [u8; 32],
    pub timestamp_ms: u64,
    pub proposer_id: [u8; 32],
}

/// Construct a block from a set of verified spends.
///
/// The block hash is `SHA-256(canonical(BlockHeader))`. Since `parent_hash`
/// is embedded in the header, the hash inherently chains blocks.
pub fn build_block(
    params: BlockParams,
    spends: Vec<SpendTransaction>,
    sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
) -> Block {
    let nullifiers: Vec<Nullifier> = spends.iter().map(|s| s.nullifier.clone()).collect();
    let transactions_root = compute_transactions_root(&nullifiers);

    let header = BlockHeader {
        zone_id: params.zone_id,
        epoch: params.epoch,
        height: params.height,
        parent_hash: params.parent_hash,
        transactions_root,
        timestamp_ms: params.timestamp_ms,
        proposer_id: params.proposer_id,
    };

    let block_hash = compute_block_hash(&header);
    let proposer_sig = sign_fn(&block_hash);

    Block {
        header,
        transactions: spends,
        block_hash,
        proposer_sig,
    }
}

/// `block_hash = SHA-256(canonical(BlockHeader))`
pub fn compute_block_hash(header: &BlockHeader) -> [u8; 32] {
    let canonical =
        grid_core::encode_canonical(header).expect("BlockHeader serialization is infallible");
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    hasher.finalize().into()
}

/// Flat hash of nullifiers: `SHA-256(n_0 || n_1 || ... || n_k)`.
/// Merkle tree can replace this later.
fn compute_transactions_root(nullifiers: &[Nullifier]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for n in nullifiers {
        hasher.update(n.as_ref());
    }
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
    fn block_hash_is_deterministic() {
        let header = BlockHeader {
            zone_id: 0,
            epoch: 1,
            height: 0,
            parent_hash: [0; 32],
            transactions_root: [0; 32],
            timestamp_ms: 1_000,
            proposer_id: [0xAA; 32],
        };
        let h1 = compute_block_hash(&header);
        let h2 = compute_block_hash(&header);
        assert_eq!(h1, h2);
    }

    #[test]
    fn block_hash_changes_with_zone() {
        let mut header = BlockHeader {
            zone_id: 0,
            epoch: 1,
            height: 0,
            parent_hash: [0; 32],
            transactions_root: [0; 32],
            timestamp_ms: 1_000,
            proposer_id: [0xAA; 32],
        };
        let h1 = compute_block_hash(&header);
        header.zone_id = 1;
        let h2 = compute_block_hash(&header);
        assert_ne!(h1, h2);
    }

    #[test]
    fn build_block_includes_all_transactions() {
        let spends = vec![dummy_spend(1), dummy_spend(2)];
        let params = BlockParams {
            zone_id: 0,
            epoch: 1,
            height: 0,
            parent_hash: [0; 32],
            timestamp_ms: 1_000,
            proposer_id: [0xAA; 32],
        };
        let block = build_block(params, spends, |hash| hash.to_vec());
        assert_eq!(block.transactions.len(), 2);
        assert_eq!(block.header.proposer_id, [0xAA; 32]);
    }

    #[test]
    fn build_block_signs_block_hash() {
        let spends = vec![dummy_spend(1)];
        let params = BlockParams {
            zone_id: 0,
            epoch: 1,
            height: 0,
            parent_hash: [0; 32],
            timestamp_ms: 1_000,
            proposer_id: [0xBB; 32],
        };
        let block = build_block(params, spends, |hash| hash.to_vec());
        assert_eq!(block.proposer_sig, block.block_hash.to_vec());
    }
}
