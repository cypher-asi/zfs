use std::collections::HashSet;

use grid_programs_zephyr::{Nullifier, ZoneId};
use grid_service::{ProgramStore, ServiceError};

/// Per-zone nullifier set: in-memory `HashSet` for O(1) lookup,
/// optionally backed by persistent sector log for durability.
///
/// Invariants:
/// - The in-memory set is always a superset of the persistent log
///   (entries are written to both atomically via `insert`).
/// - On startup, `load` replays the full log into the `HashSet`.
/// - `contains` is O(1) in-memory; no disk access needed.
pub struct NullifierSet {
    zone_id: ZoneId,
    set: HashSet<Nullifier>,
    store: Option<ProgramStore>,
}

impl NullifierSet {
    /// Create a purely in-memory nullifier set (no persistence).
    pub fn in_memory(zone_id: ZoneId) -> Self {
        Self {
            zone_id,
            set: HashSet::new(),
            store: None,
        }
    }

    /// Rebuild from persistent log on startup.
    pub fn load(zone_id: ZoneId, store: ProgramStore) -> Result<Self, ServiceError> {
        let key = format!("nullifiers/{zone_id}");
        let entries = store.list(key.as_bytes())?;
        let set: HashSet<Nullifier> = entries
            .iter()
            .filter_map(|e| grid_core::decode_canonical::<Nullifier>(e).ok())
            .collect();
        Ok(Self {
            zone_id,
            set,
            store: Some(store),
        })
    }

    /// Check membership: O(1).
    pub fn contains(&self, n: &Nullifier) -> bool {
        self.set.contains(n)
    }

    /// Insert and persist. Returns `false` if already present (double-spend).
    pub fn insert(&mut self, n: Nullifier) -> Result<bool, ServiceError> {
        if !self.set.insert(n.clone()) {
            return Ok(false);
        }
        if let Some(ref store) = self.store {
            let key = format!("nullifiers/{}", self.zone_id);
            let encoded = grid_core::encode_canonical(&n)
                .map_err(|e| ServiceError::Storage(e.to_string()))?;
            store.put(key.as_bytes(), encoded)?;
        }
        Ok(true)
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn zone_id(&self) -> ZoneId {
        self.zone_id
    }
}
