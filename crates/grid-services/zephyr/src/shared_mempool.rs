use std::collections::HashMap;
use std::sync::Arc;

use grid_programs_zephyr::{Nullifier, SpendTransaction, ZoneId};
use parking_lot::{Mutex, RwLock};

use crate::mempool::Mempool;

/// Thread-safe wrapper around per-zone mempools.
///
/// Uses a two-level locking scheme: an outer `RwLock` protects the zone map
/// (written only at epoch transitions), while each zone has its own `Mutex`.
/// This eliminates cross-zone contention -- zone 0 inserts never block
/// zone 1 proposals.
///
/// All operations are synchronous (`parking_lot` locks) since mempool work is
/// pure in-memory HashMap manipulation with no I/O, avoiding async overhead
/// in the hot consensus path.
#[derive(Clone)]
pub struct SharedMempool {
    inner: Arc<RwLock<HashMap<u32, Arc<Mutex<Mempool>>>>>,
}

impl SharedMempool {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_zone(&self, zone_id: ZoneId, max_size: usize) {
        let mut map = self.inner.write();
        map.entry(zone_id)
            .or_insert_with(|| Arc::new(Mutex::new(Mempool::new(zone_id, max_size))));
    }

    pub fn remove_zone(&self, zone_id: ZoneId) {
        self.inner.write().remove(&zone_id);
    }

    pub fn insert(&self, zone_id: ZoneId, tx: SpendTransaction) -> bool {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().insert(tx)
        } else {
            false
        }
    }

    /// Insert a batch of spends into a single zone, acquiring the zone lock once.
    pub fn insert_batch(&self, zone_id: ZoneId, txs: Vec<SpendTransaction>) -> usize {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().insert_batch(txs)
        } else {
            0
        }
    }

    /// Drain up to `max` spends for a block proposal (zero-copy move).
    /// On round timeout, call `reinsert_batch` to return un-finalized txs.
    pub fn drain_proposal(&self, zone_id: ZoneId, max: usize) -> Vec<SpendTransaction> {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().drain_proposal(max)
        } else {
            vec![]
        }
    }

    pub fn peek(&self, zone_id: ZoneId, max: usize) -> Vec<SpendTransaction> {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().peek(max)
        } else {
            vec![]
        }
    }

    /// Re-insert transactions from a proposal that never finalized.
    pub fn reinsert_batch(&self, zone_id: ZoneId, txs: Vec<SpendTransaction>) {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().reinsert_batch(txs);
        }
    }

    pub fn remove_nullifiers(&self, zone_id: ZoneId, nullifiers: &[Nullifier]) {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().remove_nullifiers(nullifiers);
        }
    }

    /// Remove all entries from a zone's mempool where the predicate returns
    /// `false`. Returns the number of entries removed.
    pub fn retain(&self, zone_id: ZoneId, keep: impl FnMut(&Nullifier) -> bool) -> usize {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().retain(keep)
        } else {
            0
        }
    }

    pub fn len(&self, zone_id: ZoneId) -> usize {
        let map = self.inner.read();
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().len()
        } else {
            0
        }
    }

    pub fn zone_sizes(&self) -> HashMap<u32, usize> {
        let map = self.inner.read();
        let mut sizes = HashMap::with_capacity(map.len());
        for (&zid, mp) in map.iter() {
            sizes.insert(zid, mp.lock().len());
        }
        sizes
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
    fn insert_and_peek() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 100);

        assert!(mp.insert(0, dummy_spend(1)));
        assert!(mp.insert(0, dummy_spend(2)));
        assert_eq!(mp.len(0), 2);

        let peeked = mp.peek(0, 10);
        assert_eq!(peeked.len(), 2);
        assert_eq!(mp.len(0), 2);
    }

    #[test]
    fn insert_returns_false_for_unknown_zone() {
        let mp = SharedMempool::new();
        assert!(!mp.insert(99, dummy_spend(1)));
    }

    #[test]
    fn remove_nullifiers_cleans_up() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 100);

        mp.insert(0, dummy_spend(1));
        mp.insert(0, dummy_spend(2));
        mp.insert(0, dummy_spend(3));

        mp.remove_nullifiers(0, &[Nullifier([1; 32]), Nullifier([3; 32])]);
        assert_eq!(mp.len(0), 1);
    }

    #[test]
    fn add_and_remove_zone() {
        let mp = SharedMempool::new();
        mp.add_zone(5, 100);
        assert!(mp.insert(5, dummy_spend(1)));
        mp.remove_zone(5);
        assert_eq!(mp.len(5), 0);
        assert!(!mp.insert(5, dummy_spend(2)));
    }

    #[test]
    fn zone_sizes_snapshot() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 100);
        mp.add_zone(1, 100);
        mp.insert(0, dummy_spend(1));
        mp.insert(0, dummy_spend(2));
        mp.insert(1, dummy_spend(3));

        let sizes = mp.zone_sizes();
        assert_eq!(sizes[&0], 2);
        assert_eq!(sizes[&1], 1);
    }

    #[tokio::test]
    async fn concurrent_insert_and_peek() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 10_000);

        let mp1 = mp.clone();
        let inserter = tokio::spawn(async move {
            for i in 0..100u8 {
                mp1.insert(0, dummy_spend(i));
            }
        });

        let mp2 = mp.clone();
        let peeker = tokio::spawn(async move {
            let mut last_len = 0;
            for _ in 0..50 {
                let peeked = mp2.peek(0, 200);
                assert!(peeked.len() >= last_len);
                last_len = peeked.len();
                tokio::task::yield_now().await;
            }
        });

        inserter.await.unwrap();
        peeker.await.unwrap();
        assert_eq!(mp.len(0), 100);
    }
}
