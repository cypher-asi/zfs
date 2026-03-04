use std::collections::{HashSet, VecDeque};

use grid_programs_zephyr::{Nullifier, SpendTransaction, ZoneId};

/// Per-zone mempool of candidate spend transactions.
///
/// Invariants:
/// - At most one spend per nullifier (prevents double-spend at mempool level)
/// - Spends are drained in FIFO order by the leader for block proposals
/// - Maximum capacity enforced to bound memory usage
///
/// Backed by a `VecDeque` for O(1) front-drain and a `HashSet` for O(1)
/// dedup. Nullifiers removed via `remove_nullifiers` are lazily evicted
/// from the queue during `drain_proposal`, avoiding the O(n) index-rebuild
/// cost of `IndexMap::drain`.
pub struct Mempool {
    zone_id: ZoneId,
    queue: VecDeque<SpendTransaction>,
    seen: HashSet<Nullifier>,
    max_size: usize,
}

impl Mempool {
    pub fn new(zone_id: ZoneId, max_size: usize) -> Self {
        Self {
            zone_id,
            queue: VecDeque::new(),
            seen: HashSet::new(),
            max_size,
        }
    }

    /// Add a spend to the mempool. Returns `false` if the nullifier is
    /// already present or the mempool is full.
    pub fn insert(&mut self, spend: SpendTransaction) -> bool {
        if self.seen.len() >= self.max_size {
            return false;
        }
        if !self.seen.insert(spend.nullifier.clone()) {
            return false;
        }
        self.queue.push_back(spend);
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
    ///
    /// Dead entries (nullifiers removed via `remove_nullifiers`) are skipped
    /// and dropped, giving amortized O(count) instead of O(total_size).
    pub fn drain_proposal(&mut self, max: usize) -> Vec<SpendTransaction> {
        let mut result = Vec::with_capacity(max);
        while result.len() < max {
            let Some(tx) = self.queue.pop_front() else {
                break;
            };
            if self.seen.remove(&tx.nullifier) {
                result.push(tx);
            }
        }
        self.maybe_compact();
        result
    }

    /// Re-insert transactions that were drained for a proposal that never
    /// finalized (e.g. round timeout). Duplicates (by nullifier) are silently
    /// skipped.
    pub fn reinsert_batch(&mut self, txs: Vec<SpendTransaction>) {
        for tx in txs {
            if self.seen.len() >= self.max_size {
                break;
            }
            if self.seen.insert(tx.nullifier.clone()) {
                self.queue.push_back(tx);
            }
        }
    }

    /// Return clones of up to `max` spends without removing them.
    pub fn peek(&self, max: usize) -> Vec<SpendTransaction> {
        self.queue
            .iter()
            .filter(|tx| self.seen.contains(&tx.nullifier))
            .take(max)
            .cloned()
            .collect()
    }

    /// Check if a nullifier is already in the mempool.
    pub fn contains_nullifier(&self, nullifier: &Nullifier) -> bool {
        self.seen.contains(nullifier)
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    pub fn zone_id(&self) -> ZoneId {
        self.zone_id
    }

    /// Remove all spends whose nullifiers are in the given set (after finalization).
    ///
    /// Only removes from the `seen` set; dead entries are lazily evicted from
    /// the queue during `drain_proposal`.
    pub fn remove_nullifiers(&mut self, nullifiers: &[Nullifier]) {
        for n in nullifiers {
            self.seen.remove(n);
        }
        self.maybe_compact();
    }

    /// Remove all entries where the predicate returns `false`.
    /// Returns the number of entries removed.
    pub fn retain(&mut self, mut keep: impl FnMut(&Nullifier) -> bool) -> usize {
        let before = self.seen.len();
        self.seen.retain(|k| keep(k));
        let removed = before - self.seen.len();
        if removed > 0 {
            self.queue.retain(|tx| self.seen.contains(&tx.nullifier));
        }
        removed
    }

    /// Compact the queue when dead entries exceed 2× live entries, preventing
    /// unbounded growth from lazy removal.
    fn maybe_compact(&mut self) {
        if self.queue.len() > self.seen.len().saturating_mul(3).max(4096) {
            self.queue.retain(|tx| self.seen.contains(&tx.nullifier));
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

    #[test]
    fn drain_skips_removed_nullifiers() {
        let mut mp = Mempool::new(0, 100);
        mp.insert(dummy_spend(1));
        mp.insert(dummy_spend(2));
        mp.insert(dummy_spend(3));

        mp.remove_nullifiers(&[Nullifier([1; 32]), Nullifier([2; 32])]);

        let drained = mp.drain_proposal(10);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].nullifier, Nullifier([3; 32]));
        assert!(mp.is_empty());
    }

    #[test]
    fn reinsert_after_timeout() {
        let mut mp = Mempool::new(0, 100);
        mp.insert(dummy_spend(1));
        mp.insert(dummy_spend(2));

        let drained = mp.drain_proposal(2);
        assert_eq!(mp.len(), 0);

        mp.reinsert_batch(drained);
        assert_eq!(mp.len(), 2);

        let re_drained = mp.drain_proposal(2);
        assert_eq!(re_drained.len(), 2);
    }
}
