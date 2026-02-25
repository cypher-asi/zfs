use sha2::{Digest, Sha256};
use zfs_core::{ProgramId, SectorId};

use crate::error::StorageError;
use crate::rocks::{RocksStorage, CF_SECTORS};
use crate::sector_traits::{SectorBatchEntry, SectorPutResult, SectorStorageStats, SectorStore};

impl SectorStore for RocksStorage {
    fn put(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
        payload: &[u8],
        overwrite: bool,
        expected_hash: Option<&[u8]>,
    ) -> Result<(), StorageError> {
        let key = build_sector_key(program_id, sector_id);
        let cf = self.cf_handle(CF_SECTORS)?;

        if !overwrite {
            if self.db().get_cf(cf, &key)?.is_some() {
                return Err(StorageError::SlotOccupied);
            }
            self.db().put_cf(cf, &key, payload)?;
            return Ok(());
        }

        cas_put(self, cf, &key, payload, expected_hash)
    }

    fn get(
        &self,
        program_id: &ProgramId,
        sector_id: &SectorId,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let key = build_sector_key(program_id, sector_id);
        let cf = self.cf_handle(CF_SECTORS)?;
        Ok(self.db().get_cf(cf, &key)?)
    }

    fn batch_put(
        &self,
        program_id: &ProgramId,
        entries: &[SectorBatchEntry],
    ) -> Result<Vec<SectorPutResult>, StorageError> {
        let mut results = Vec::with_capacity(entries.len());
        for (sector_id, payload, overwrite, expected_hash) in entries {
            let hash_ref = expected_hash.as_deref();
            match self.put(program_id, sector_id, payload, *overwrite, hash_ref) {
                Ok(()) => results.push(SectorPutResult {
                    ok: true,
                    error: None,
                }),
                Err(e) => results.push(SectorPutResult {
                    ok: false,
                    error: Some(e),
                }),
            }
        }
        Ok(results)
    }

    fn batch_get(
        &self,
        program_id: &ProgramId,
        sector_ids: &[SectorId],
    ) -> Result<Vec<Option<Vec<u8>>>, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        let mut results = Vec::with_capacity(sector_ids.len());
        for sid in sector_ids {
            let key = build_sector_key(program_id, sid);
            results.push(self.db().get_cf(cf, &key)?);
        }
        Ok(results)
    }

    fn sector_stats(&self) -> Result<SectorStorageStats, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        let mut count = 0u64;
        let mut size = 0u64;
        let iter = self.db().iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_k, v) = item?;
            count += 1;
            size += v.len() as u64;
        }
        Ok(SectorStorageStats {
            sector_count: count,
            sector_size_bytes: size,
        })
    }

    fn list_programs(&self) -> Result<Vec<ProgramId>, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        let mut programs = Vec::new();
        let mut last_pid: Option<[u8; 32]> = None;
        let iter = self.db().iterator_cf(cf, rocksdb::IteratorMode::Start);
        for item in iter {
            let (key, _) = item?;
            if key.len() < 32 {
                continue;
            }
            let pid_bytes: [u8; 32] = key[..32].try_into().unwrap();
            if last_pid.as_ref() != Some(&pid_bytes) {
                programs.push(ProgramId::from(pid_bytes));
                last_pid = Some(pid_bytes);
            }
        }
        Ok(programs)
    }

    fn list_sectors(&self, program_id: &ProgramId) -> Result<Vec<SectorId>, StorageError> {
        let cf = self.cf_handle(CF_SECTORS)?;
        let prefix = program_id.as_bytes();
        let iter = self.db().prefix_iterator_cf(cf, prefix);
        let mut sectors = Vec::new();
        for item in iter {
            let (key, _) = item?;
            if key.len() < 32 || &key[..32] != prefix {
                break;
            }
            sectors.push(SectorId::from_bytes(key[32..].to_vec()));
        }
        Ok(sectors)
    }
}

fn build_sector_key(program_id: &ProgramId, sector_id: &SectorId) -> Vec<u8> {
    let pid = program_id.as_bytes();
    let sid = sector_id.as_bytes();
    let mut key = Vec::with_capacity(pid.len() + sid.len());
    key.extend_from_slice(pid);
    key.extend_from_slice(sid);
    key
}

fn cas_put(
    storage: &RocksStorage,
    cf: &rocksdb::ColumnFamily,
    key: &[u8],
    payload: &[u8],
    expected_hash: Option<&[u8]>,
) -> Result<(), StorageError> {
    if let Some(expected) = expected_hash {
        match storage.db().get_cf(cf, key)? {
            Some(current) => {
                let actual_hash = Sha256::digest(&current);
                if actual_hash.as_slice() != expected {
                    return Err(StorageError::ConditionFailed);
                }
            }
            None => {
                return Err(StorageError::ConditionFailed);
            }
        }
    }
    storage.db().put_cf(cf, key, payload)?;
    Ok(())
}
