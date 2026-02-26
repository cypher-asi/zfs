use zfs_core::{ProgramId, SectorId};

use crate::error::StorageError;
use crate::rocks::{RocksStorage, CF_PROOFS, CF_SECTORS};
use crate::sector_traits::{SectorStorageStats, SectorStore};

const PID_LEN: usize = 32;
const SID_LEN: usize = 32;
const IDX_LEN: usize = 8;
const PREFIX_LEN: usize = PID_LEN + SID_LEN;
const KEY_LEN: usize = PREFIX_LEN + IDX_LEN;

impl SectorStore for RocksStorage {
    fn append(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        entry: &[u8],
    ) -> Result<u64, StorageError> {
        validate_sector_id(sector_id)?;
        let cf = self.cf_handle(CF_SECTORS)?;
        let prefix = build_sector_prefix(program_id, sector_id);
        let next_index = scan_max_index(self, cf, &prefix)?;
        let key = build_entry_key(program_id, sector_id, next_index);
        self.db().put_cf(cf, &key, entry)?;
        Ok(next_index)
    }

    fn insert_at(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
        entry: &[u8],
    ) -> Result<bool, StorageError> {
        validate_sector_id(sector_id)?;
        let cf = self.cf_handle(CF_SECTORS)?;
        let key = build_entry_key(program_id, sector_id, index);
        if self.db().get_cf(cf, &key)?.is_some() {
            return Ok(false);
        }
        self.db().put_cf(cf, &key, entry)?;
        Ok(true)
    }

    fn read_log(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        from_index: u64,
        max_entries: usize,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        validate_sector_id(sector_id)?;
        let cf = self.cf_handle(CF_SECTORS)?;
        let prefix = build_sector_prefix(program_id, sector_id);
        let start = build_entry_key(program_id, sector_id, from_index);
        read_entries(self, cf, &prefix, &start, max_entries)
    }

    fn log_length(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
    ) -> Result<u64, StorageError> {
        validate_sector_id(sector_id)?;
        let cf = self.cf_handle(CF_SECTORS)?;
        let prefix = build_sector_prefix(program_id, sector_id);
        scan_max_index(self, cf, &prefix)
    }

    fn sector_stats(&self) -> Result<SectorStorageStats, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        collect_stats(self, cf)
    }

    fn list_programs(&self) -> Result<Vec<ProgramId>, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        collect_programs(self, cf)
    }

    fn list_sectors(&self, program_id: &ProgramId) -> Result<Vec<SectorId>, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        collect_sectors(self, cf, program_id)
    }

    fn get_entry(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        validate_sector_id(sector_id)?;
        let cf = self.cf_handle(CF_SECTORS)?;
        let key = build_entry_key(program_id, sector_id, index);
        Ok(self.db().get_cf(cf, &key)?)
    }

    fn store_proof(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
        proof: &[u8],
    ) -> Result<(), StorageError> {
        let cf = self.cf_handle(CF_PROOFS)?;
        let key = build_entry_key(program_id, sector_id, index);
        self.db().put_cf(cf, &key, proof)?;
        Ok(())
    }

    fn get_proof(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        index: u64,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let cf = self.cf_handle(CF_PROOFS)?;
        let key = build_entry_key(program_id, sector_id, index);
        Ok(self.db().get_cf(cf, &key)?)
    }
}

fn validate_sector_id(sector_id: &SectorId) -> Result<(), StorageError> {
    if sector_id.as_bytes().len() != SID_LEN {
        return Err(StorageError::Decode(format!(
            "sector ID must be {SID_LEN} bytes, got {}",
            sector_id.as_bytes().len()
        )));
    }
    Ok(())
}

fn build_sector_prefix(program_id: &ProgramId, sector_id: &SectorId) -> Vec<u8> {
    let mut prefix = Vec::with_capacity(PREFIX_LEN);
    prefix.extend_from_slice(program_id.as_bytes());
    prefix.extend_from_slice(sector_id.as_bytes());
    prefix
}

fn build_entry_key(program_id: &ProgramId, sector_id: &SectorId, index: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(KEY_LEN);
    key.extend_from_slice(program_id.as_bytes());
    key.extend_from_slice(sector_id.as_bytes());
    key.extend_from_slice(&index.to_be_bytes());
    key
}

/// Reverse-seek to find the highest stored index for a sector prefix.
/// Returns `max_index + 1` (i.e. the next available index), or 0 if empty.
fn scan_max_index(
    storage: &RocksStorage,
    cf: &rocksdb::ColumnFamily,
    prefix: &[u8],
) -> Result<u64, StorageError> {
    let mut seek_key = prefix.to_vec();
    seek_key.extend_from_slice(&u64::MAX.to_be_bytes());
    let mode = rocksdb::IteratorMode::From(&seek_key, rocksdb::Direction::Reverse);
    let mut iter = storage.db().iterator_cf(cf, mode);
    match iter.next() {
        Some(Ok((key, _))) if key.starts_with(prefix) => {
            if key.len() != prefix.len() + IDX_LEN {
                return Err(StorageError::Decode(
                    "unexpected key length in sector log".into(),
                ));
            }
            // slice length validated above
            let idx: [u8; IDX_LEN] = key[prefix.len()..]
                .try_into()
                .expect("slice is exactly IDX_LEN bytes");
            Ok(u64::from_be_bytes(idx) + 1)
        }
        Some(Err(e)) => Err(StorageError::RocksDb(e)),
        _ => Ok(0),
    }
}

fn read_entries(
    storage: &RocksStorage,
    cf: &rocksdb::ColumnFamily,
    prefix: &[u8],
    start_key: &[u8],
    max_entries: usize,
) -> Result<Vec<Vec<u8>>, StorageError> {
    let mode = rocksdb::IteratorMode::From(start_key, rocksdb::Direction::Forward);
    let iter = storage.db().iterator_cf(cf, mode);
    let mut entries = Vec::with_capacity(max_entries.min(64));
    for item in iter {
        if entries.len() >= max_entries {
            break;
        }
        let (key, value) = item?;
        if !key.starts_with(prefix) {
            break;
        }
        entries.push(value.to_vec());
    }
    Ok(entries)
}

fn collect_stats(
    storage: &RocksStorage,
    cf: &rocksdb::ColumnFamily,
) -> Result<SectorStorageStats, StorageError> {
    let mut entry_count = 0u64;
    let mut size = 0u64;
    let mut sector_count = 0u64;
    let mut last_sector: Option<Vec<u8>> = None;

    let iter = storage.db().iterator_cf(cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, v) = item?;
        if key.len() < PREFIX_LEN {
            continue;
        }
        entry_count += 1;
        size += v.len() as u64;

        let sector_key = &key[..PREFIX_LEN];
        let is_new = last_sector
            .as_ref()
            .is_none_or(|prev| prev.as_slice() != sector_key);
        if is_new {
            sector_count += 1;
            last_sector = Some(sector_key.to_vec());
        }
    }

    Ok(SectorStorageStats {
        sector_count,
        entry_count,
        sector_size_bytes: size,
    })
}

fn collect_programs(
    storage: &RocksStorage,
    cf: &rocksdb::ColumnFamily,
) -> Result<Vec<ProgramId>, StorageError> {
    let mut programs = Vec::new();
    let mut last_pid: Option<[u8; PID_LEN]> = None;
    let iter = storage.db().iterator_cf(cf, rocksdb::IteratorMode::Start);
    for item in iter {
        let (key, _) = item?;
        if key.len() < PID_LEN {
            continue;
        }
        // length validated above
        let pid: [u8; PID_LEN] = key[..PID_LEN]
            .try_into()
            .expect("slice is exactly PID_LEN bytes");
        if last_pid.as_ref() != Some(&pid) {
            programs.push(ProgramId::from(pid));
            last_pid = Some(pid);
        }
    }
    Ok(programs)
}

fn collect_sectors(
    storage: &RocksStorage,
    cf: &rocksdb::ColumnFamily,
    program_id: &ProgramId,
) -> Result<Vec<SectorId>, StorageError> {
    let prefix = program_id.as_bytes().as_slice();
    let iter = storage.db().prefix_iterator_cf(cf, prefix);
    let mut sectors = Vec::new();
    let mut last_sid: Option<[u8; SID_LEN]> = None;
    for item in iter {
        let (key, _) = item?;
        if key.len() < PREFIX_LEN || &key[..PID_LEN] != prefix {
            break;
        }
        // length validated above
        let sid: [u8; SID_LEN] = key[PID_LEN..PREFIX_LEN]
            .try_into()
            .expect("slice is exactly SID_LEN bytes");
        if last_sid.as_ref() != Some(&sid) {
            sectors.push(SectorId::from_bytes(sid.to_vec()));
            last_sid = Some(sid);
        }
    }
    Ok(sectors)
}
