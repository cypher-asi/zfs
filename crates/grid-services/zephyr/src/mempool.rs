use grid_programs_zephyr::{Nullifier, SpendTransaction, ZoneId};
use indexmap::IndexMap;

/// Per-zone mempool of candidate spend transactions.
///
/// Invariants:
/// - At most one spend per nullifier (prevents double-spend at mempool level)
/// - Spends are drained in FIFO order by the leader for block proposals
/// - Maximum capacity enforced to bound memory usage
///
/// Backed by `IndexMap` for O(1) insert, O(1) contains, and O(1) removal
/// by nullifier (via `swap_remove`), while preserving insertion order for
/// `peek` and `drain`.
pub struct Mempool {
    zone_id: ZoneId,
    queue: IndexMap<Nullifier, SpendTransaction>,
    max_size: usize,
}

impl Mempool {
    pub fn new(zone_id: ZoneId, max_size: usize) -> Self {
        Self {
            zone_id,
            queue: IndexMap::new(),
            max_size,
        }
    }

    /// Add a spend to the mempool. Returns `false` if the nullifier is
    /// already present or the mempool is full.
    pub fn insert(&mut self, spend: SpendTransaction) -> bool {
        if self.queue.len() >= self.max_size {
            return false;
        }
        if self.queue.contains_key(&spend.nullifier) {
            return false;
        }
        self.queue.insert(spend.nullifier.clone(), spend);
        true
    }

    /// Insert a batch of spends, acquiring no additional locks.
    /// Returns the number of successfully inserted spends.
    pub fn insert_batch(&mut self, spends: Vec<SpendTransaction>) -> usize {
        let mut inserted = 0;
        for spend in spends {
            if self.insert(spend) {
                inserted += 1;
            }
        }
        inserted
    }

    /// Drain up to `max` spends from the mempool (FIFO order) for a block
    /// proposal. Moves transactions out without cloning. On round timeout the
    /// caller should call `reinsert_batch` to return un-finalized transactions.
    pub fn drain_proposal(&mut self, max: usize) -> Vec<SpendTransaction> {
        let count = max.min(self.queue.len());
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some((_nullifier, spend)) = self.queue.shift_remove_index(0) {
                result.push(spend);
            }
        }
        result
    }

    /// Re-insert transactions that were drained for a proposal that never
    /// finalized (e.g. round timeout). Duplicates (by nullifier) are silently
    /// skipped.
    pub fn reinsert_batch(&mut self, txs: Vec<SpendTransaction>) {
        for tx in txs {
            if self.queue.len() >= self.max_size {
                break;
            }
            self.queue.entry(tx.nullifier.clone()).or_insert(tx);
        }
    }

    /// Return clones of up to `max` spends without removing them.
    pub fn peek(&self, max: usize) -> Vec<SpendTransaction> {
        self.queue.values().take(max).cloned().collect()
    }

    /// Check if a nullifier is already in the mempool.
    pub fn contains_nullifier(&self, nullifier: &Nullifier) -> bool {
        self.queue.contains_key(nullifier)
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn zone_id(&self) -> ZoneId {
        self.zone_id
    }

    /// Remove all spends whose nullifiers are in the given set (after finalization).
    ///
    /// Uses `swap_remove` for O(1) per nullifier. This reorders the tail of
    /// the map but that is acceptable since block proposal ordering within a
    /// block is not consensus-critical.
    pub fn remove_nullifiers(&mut self, nullifiers: &[Nullifier]) {
        for n in nullifiers {
            self.queue.swap_remove(n);
        }
    }
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
    fn insert_and_drain() {
        let mut mp = Mempool::new(0, 100);
        assert!(mp.insert(dummy_spend(1)));
        assert!(mp.insert(dummy_spend(2)));
        assert_eq!(mp.len(), 2);

        let drained = mp.drain_proposal(1);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].nullifier, Nullifier([1; 32]));
        assert_eq!(mp.len(), 1);
    }

    #[test]
    fn rejects_duplicate_nullifier() {
        let mut mp = Mempool::new(0, 100);
        assert!(mp.insert(dummy_spend(1)));
        assert!(!mp.insert(dummy_spend(1)));
        assert_eq!(mp.len(), 1);
    }

    #[test]
    fn rejects_when_full() {
        let mut mp = Mempool::new(0, 2);
        assert!(mp.insert(dummy_spend(1)));
        assert!(mp.insert(dummy_spend(2)));
        assert!(!mp.insert(dummy_spend(3)));
    }

    #[test]
    fn contains_nullifier_check() {
        let mut mp = Mempool::new(0, 100);
        let n = Nullifier([0xAA; 32]);
        assert!(!mp.contains_nullifier(&n));
        mp.insert(dummy_spend(0xAA));
        assert!(mp.contains_nullifier(&n));
    }

    #[test]
    fn drain_more_than_available() {
        let mut mp = Mempool::new(0, 100);
        mp.insert(dummy_spend(1));
        let drained = mp.drain_proposal(10);
        assert_eq!(drained.len(), 1);
        assert!(mp.is_empty());
    }

    #[test]
    fn remove_nullifiers_after_finalization() {
        let mut mp = Mempool::new(0, 100);
        mp.insert(dummy_spend(1));
        mp.insert(dummy_spend(2));
        mp.insert(dummy_spend(3));

        mp.remove_nullifiers(&[Nullifier([1; 32]), Nullifier([3; 32])]);
        assert_eq!(mp.len(), 1);
        assert!(!mp.contains_nullifier(&Nullifier([1; 32])));
        assert!(mp.contains_nullifier(&Nullifier([2; 32])));
        assert!(!mp.contains_nullifier(&Nullifier([3; 32])));
    }

    #[test]
    fn fifo_ordering() {
        let mut mp = Mempool::new(0, 100);
        for i in 0..5 {
            mp.insert(dummy_spend(i));
        }
        let drained = mp.drain_proposal(5);
        for (i, spend) in drained.iter().enumerate() {
            assert_eq!(spend.nullifier, Nullifier([i as u8; 32]));
        }
    }

    #[test]
    fn peek_does_not_remove() {
        let mut mp = Mempool::new(0, 100);
        mp.insert(dummy_spend(1));
        mp.insert(dummy_spend(2));

        let peeked = mp.peek(2);
        assert_eq!(peeked.len(), 2);
        assert_eq!(mp.len(), 2, "peek must not remove items");

        let peeked_one = mp.peek(1);
        assert_eq!(peeked_one.len(), 1);
        assert_eq!(peeked_one[0].nullifier, Nullifier([1; 32]));
    }
}
