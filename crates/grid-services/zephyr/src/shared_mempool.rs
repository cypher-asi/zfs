use std::collections::HashMap;
use std::sync::Arc;

use grid_programs_zephyr::{Nullifier, SpendTransaction, ZoneId};
use tokio::sync::{Mutex, RwLock};

use crate::mempool::Mempool;

/// Thread-safe wrapper around per-zone mempools.
///
/// Uses a two-level locking scheme: an outer `RwLock` protects the zone map
/// (written only at epoch transitions), while each zone has its own `Mutex`.
/// This eliminates cross-zone contention -- zone 0 inserts never block
/// zone 1 proposals.
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

    pub async fn add_zone(&self, zone_id: ZoneId, max_size: usize) {
        let mut map = self.inner.write().await;
        map.entry(zone_id)
            .or_insert_with(|| Arc::new(Mutex::new(Mempool::new(zone_id, max_size))));
    }

    pub async fn remove_zone(&self, zone_id: ZoneId) {
        self.inner.write().await.remove(&zone_id);
    }

    pub async fn insert(&self, zone_id: ZoneId, tx: SpendTransaction) -> bool {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.insert(tx)
        } else {
            false
        }
    }

    /// Insert a batch of spends into a single zone, acquiring the zone lock once.
    pub async fn insert_batch(&self, zone_id: ZoneId, txs: Vec<SpendTransaction>) -> usize {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.insert_batch(txs)
        } else {
            0
        }
    }

    /// Drain up to `max` spends for a block proposal (zero-copy move).
    /// On round timeout, call `reinsert_batch` to return un-finalized txs.
    pub async fn drain_proposal(&self, zone_id: ZoneId, max: usize) -> Vec<SpendTransaction> {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.drain_proposal(max)
        } else {
            vec![]
        }
    }

    pub async fn peek(&self, zone_id: ZoneId, max: usize) -> Vec<SpendTransaction> {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.peek(max)
        } else {
            vec![]
        }
    }

    /// Re-insert transactions from a proposal that never finalized.
    pub async fn reinsert_batch(&self, zone_id: ZoneId, txs: Vec<SpendTransaction>) {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.reinsert_batch(txs);
        }
    }

    pub async fn remove_nullifiers(&self, zone_id: ZoneId, nullifiers: &[Nullifier]) {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.remove_nullifiers(nullifiers);
        }
    }

    pub async fn len(&self, zone_id: ZoneId) -> usize {
        let map = self.inner.read().await;
        if let Some(mp) = map.get(&zone_id) {
            mp.lock().await.len()
        } else {
            0
        }
    }

    pub async fn zone_sizes(&self) -> HashMap<u32, usize> {
        let map = self.inner.read().await;
        let mut sizes = HashMap::with_capacity(map.len());
        for (&zid, mp) in map.iter() {
            sizes.insert(zid, mp.lock().await.len());
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

    #[tokio::test]
    async fn insert_and_peek() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 100).await;

        assert!(mp.insert(0, dummy_spend(1)).await);
        assert!(mp.insert(0, dummy_spend(2)).await);
        assert_eq!(mp.len(0).await, 2);

        let peeked = mp.peek(0, 10).await;
        assert_eq!(peeked.len(), 2);
        assert_eq!(mp.len(0).await, 2);
    }

    #[tokio::test]
    async fn insert_returns_false_for_unknown_zone() {
        let mp = SharedMempool::new();
        assert!(!mp.insert(99, dummy_spend(1)).await);
    }

    #[tokio::test]
    async fn remove_nullifiers_cleans_up() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 100).await;

        mp.insert(0, dummy_spend(1)).await;
        mp.insert(0, dummy_spend(2)).await;
        mp.insert(0, dummy_spend(3)).await;

        mp.remove_nullifiers(0, &[Nullifier([1; 32]), Nullifier([3; 32])])
            .await;
        assert_eq!(mp.len(0).await, 1);
    }

    #[tokio::test]
    async fn add_and_remove_zone() {
        let mp = SharedMempool::new();
        mp.add_zone(5, 100).await;
        assert!(mp.insert(5, dummy_spend(1)).await);
        mp.remove_zone(5).await;
        assert_eq!(mp.len(5).await, 0);
        assert!(!mp.insert(5, dummy_spend(2)).await);
    }

    #[tokio::test]
    async fn zone_sizes_snapshot() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 100).await;
        mp.add_zone(1, 100).await;
        mp.insert(0, dummy_spend(1)).await;
        mp.insert(0, dummy_spend(2)).await;
        mp.insert(1, dummy_spend(3)).await;

        let sizes = mp.zone_sizes().await;
        assert_eq!(sizes[&0], 2);
        assert_eq!(sizes[&1], 1);
    }

    #[tokio::test]
    async fn concurrent_insert_and_peek() {
        let mp = SharedMempool::new();
        mp.add_zone(0, 10_000).await;

        let mp1 = mp.clone();
        let inserter = tokio::spawn(async move {
            for i in 0..100u8 {
                mp1.insert(0, dummy_spend(i)).await;
            }
        });

        let mp2 = mp.clone();
        let peeker = tokio::spawn(async move {
            let mut last_len = 0;
            for _ in 0..50 {
                let peeked = mp2.peek(0, 200).await;
                assert!(peeked.len() >= last_len);
                last_len = peeked.len();
                tokio::task::yield_now().await;
            }
        });

        inserter.await.unwrap();
        peeker.await.unwrap();
        assert_eq!(mp.len(0).await, 100);
    }
}
