use std::collections::{HashMap, VecDeque};

use grid_programs_zephyr::{Nullifier, SpendTransaction, ZoneId};

/// Per-zone mempool of candidate spend transactions.
///
/// Invariants:
/// - At most one spend per nullifier (prevents double-spend at mempool level)
/// - Spends are drained in FIFO order by the leader for block proposals
/// - Maximum capacity enforced to bound memory usage
pub struct Mempool {
    zone_id: ZoneId,
    queue: VecDeque<SpendTransaction>,
    seen_nullifiers: HashMap<Nullifier, usize>,
    max_size: usize,
}

impl Mempool {
    pub fn new(zone_id: ZoneId, max_size: usize) -> Self {
        Self {
            zone_id,
            queue: VecDeque::new(),
            seen_nullifiers: HashMap::new(),
            max_size,
        }
    }

    /// Add a spend to the mempool. Returns `false` if the nullifier is
    /// already present or the mempool is full.
    pub fn insert(&mut self, spend: SpendTransaction) -> bool {
        if self.queue.len() >= self.max_size {
            return false;
        }
        if self.seen_nullifiers.contains_key(&spend.nullifier) {
            return false;
        }
        self.seen_nullifiers
            .insert(spend.nullifier.clone(), self.queue.len());
        self.queue.push_back(spend);
        true
    }

    /// Drain up to `max` spends from the mempool (FIFO order).
    pub fn drain(&mut self, max: usize) -> Vec<SpendTransaction> {
        let count = max.min(self.queue.len());
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(spend) = self.queue.pop_front() {
                self.seen_nullifiers.remove(&spend.nullifier);
                result.push(spend);
            }
        }
        self.reindex();
        result
    }

    /// Check if a nullifier is already in the mempool.
    pub fn contains_nullifier(&self, nullifier: &Nullifier) -> bool {
        self.seen_nullifiers.contains_key(nullifier)
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
    pub fn remove_nullifiers(&mut self, nullifiers: &[Nullifier]) {
        for n in nullifiers {
            self.seen_nullifiers.remove(n);
        }
        self.queue.retain(|s| !nullifiers.contains(&s.nullifier));
        self.reindex();
    }

    fn reindex(&mut self) {
        self.seen_nullifiers.clear();
        for (i, spend) in self.queue.iter().enumerate() {
            self.seen_nullifiers.insert(spend.nullifier.clone(), i);
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

        let drained = mp.drain(1);
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
        let drained = mp.drain(10);
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
        let drained = mp.drain(5);
        for (i, spend) in drained.iter().enumerate() {
            assert_eq!(spend.nullifier, Nullifier([i as u8; 32]));
        }
    }
}
