use std::collections::HashMap;

use grid_programs_zephyr::ZoneId;

/// Tracks the current head hash for each zone.
///
/// Provides fast in-memory access to zone heads for consensus
/// validation (checking `parent_hash` in block proposals).
pub struct ZoneHead {
    heads: HashMap<ZoneId, [u8; 32]>,
}

impl ZoneHead {
    pub fn new() -> Self {
        Self {
            heads: HashMap::new(),
        }
    }

    pub fn get(&self, zone_id: ZoneId) -> Option<&[u8; 32]> {
        self.heads.get(&zone_id)
    }

    /// Returns the zone head or a zero hash if no blocks have been finalized.
    pub fn get_or_genesis(&self, zone_id: ZoneId) -> [u8; 32] {
        self.heads.get(&zone_id).copied().unwrap_or([0u8; 32])
    }

    pub fn set(&mut self, zone_id: ZoneId, head: [u8; 32]) {
        self.heads.insert(zone_id, head);
    }

    pub fn contains(&self, zone_id: ZoneId) -> bool {
        self.heads.contains_key(&zone_id)
    }

    pub fn remove(&mut self, zone_id: ZoneId) {
        self.heads.remove(&zone_id);
    }
}

impl Default for ZoneHead {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_or_genesis_returns_zero_for_unknown() {
        let zh = ZoneHead::new();
        assert_eq!(zh.get_or_genesis(42), [0u8; 32]);
    }

    #[test]
    fn set_and_get() {
        let mut zh = ZoneHead::new();
        let head = [0xAB; 32];
        zh.set(5, head);
        assert_eq!(zh.get(5), Some(&head));
        assert_eq!(zh.get_or_genesis(5), head);
    }

    #[test]
    fn remove_clears_head() {
        let mut zh = ZoneHead::new();
        zh.set(3, [1; 32]);
        assert!(zh.contains(3));
        zh.remove(3);
        assert!(!zh.contains(3));
        assert_eq!(zh.get(3), None);
    }
}
