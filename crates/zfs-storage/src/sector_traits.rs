use zfs_core::{ProgramId, SectorId};

use crate::StorageError;

/// Statistics for the sector column family.
#[derive(Debug, Clone, Default)]
pub struct SectorStorageStats {
    /// Number of distinct sectors (unique program_id + sector_id pairs).
    pub sector_count: u64,
    /// Total number of log entries across all sectors.
    pub entry_count: u64,
    /// Approximate size of sector data in bytes.
    pub sector_size_bytes: u64,
}

/// Append-only log storage for encrypted sector entries.
///
/// Each sector is an ordered sequence of entries identified by
/// `(program_id, sector_id)`. Entries are indexed starting at 0.
/// Key layout: `pid(32B) || sid(32B) || index(8B big-endian)`.
pub trait SectorStore {
    /// Append an entry to the sector log. Returns the assigned index.
    fn append(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        entry: &[u8],
    ) -> Result<u64, StorageError>;

    /// Store an entry at a specific index (for gossip replication).
    /// Returns `true` if stored, `false` if the index already exists.
    fn insert_at(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
        entry: &[u8],
    ) -> Result<bool, StorageError>;

    /// Read log entries starting from `from_index`, up to `max_entries`.
    fn read_log(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        from_index: u64,
        max_entries: usize,
    ) -> Result<Vec<Vec<u8>>, StorageError>;

    /// Return the logical length of the sector log (max stored index + 1).
    fn log_length(&self, program_id: &ProgramId, sector_id: &SectorId)
        -> Result<u64, StorageError>;

    /// Sector storage statistics.
    fn sector_stats(&self) -> Result<SectorStorageStats, StorageError>;

    /// List all distinct program IDs that have at least one stored sector.
    fn list_programs(&self) -> Result<Vec<ProgramId>, StorageError>;

    /// List all sector IDs stored for a given program.
    fn list_sectors(&self, program_id: &ProgramId) -> Result<Vec<SectorId>, StorageError>;

    /// Persist a proof blob for a given sector log entry.
    fn store_proof(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
        proof: &[u8],
    ) -> Result<(), StorageError>;

    /// Retrieve the proof blob for a given sector log entry, if any.
    fn get_proof(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
    ) -> Result<Option<Vec<u8>>, StorageError>;
}
