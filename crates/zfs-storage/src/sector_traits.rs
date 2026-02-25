use zfs_core::{ProgramId, SectorId};

use crate::StorageError;

/// Entry for batch sector put: `(sector_id, payload, overwrite, expected_hash)`.
pub type SectorBatchEntry = (SectorId, Vec<u8>, bool, Option<Vec<u8>>);

/// Per-entry result of a sector batch put operation.
#[derive(Debug)]
pub struct SectorPutResult {
    pub ok: bool,
    pub error: Option<StorageError>,
}

/// Statistics for the sector column family.
#[derive(Debug, Clone, Default)]
pub struct SectorStorageStats {
    /// Number of sector entries stored.
    pub sector_count: u64,
    /// Approximate size of sector data in bytes.
    pub sector_size_bytes: u64,
}

/// Key-value storage for encrypted sector payloads.
///
/// Each sector is identified by `(program_id, sector_id)`. Supports
/// write-once semantics, mutable overwrites, and compare-and-swap.
pub trait SectorStore {
    /// Store a sector payload.
    ///
    /// - `overwrite = false`: write-once; fails with `SlotOccupied` if key exists.
    /// - `overwrite = true, expected_hash = None`: unconditional overwrite.
    /// - `overwrite = true, expected_hash = Some(h)`: CAS — overwrites only if
    ///   the SHA-256 of the current value matches `h`.
    fn put(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        payload: &[u8],
        overwrite: bool,
        expected_hash: Option<&[u8]>,
    ) -> Result<(), StorageError>;

    /// Fetch a sector payload by program and sector ID.
    fn get(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
    ) -> Result<Option<Vec<u8>>, StorageError>;

    /// Store multiple sectors in a batch. Returns one result per entry.
    fn batch_put(
        &self,
        program_id: &ProgramId,
        entries: &[SectorBatchEntry],
    ) -> Result<Vec<SectorPutResult>, StorageError>;

    /// Fetch multiple sectors in a batch.
    fn batch_get(
        &self,
        program_id: &ProgramId,
        sector_ids: &[SectorId],
    ) -> Result<Vec<Option<Vec<u8>>>, StorageError>;

    /// Sector storage statistics.
    fn sector_stats(&self) -> Result<SectorStorageStats, StorageError>;

    /// List all distinct program IDs that have at least one stored sector.
    fn list_programs(&self) -> Result<Vec<ProgramId>, StorageError>;

    /// List all sector IDs stored for a given program.
    fn list_sectors(&self, program_id: &ProgramId) -> Result<Vec<SectorId>, StorageError>;
}
