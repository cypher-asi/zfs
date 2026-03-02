use grid_programs_zephyr::{FinalityCertificate, ZoneId};
use grid_service::{ProgramStore, ServiceError};

/// Append-only log of finalized blocks for a zone.
///
/// Maps directly to a sector log — one entry per certified block.
/// The latest entry represents the current zone head.
pub struct ZoneLog {
    zone_id: ZoneId,
    store: ProgramStore,
}

impl ZoneLog {
    pub fn new(zone_id: ZoneId, store: ProgramStore) -> Self {
        Self { zone_id, store }
    }

    /// Append a finalized certificate to the zone log.
    pub fn append_block(&self, cert: &FinalityCertificate) -> Result<(), ServiceError> {
        let key = format!("zone_log/{}", self.zone_id);
        let encoded =
            grid_core::encode_canonical(cert).map_err(|e| ServiceError::Storage(e.to_string()))?;
        self.store.put(key.as_bytes(), encoded)
    }

    /// Get the latest zone head hash, or `None` if no blocks have been finalized.
    pub fn head(&self) -> Result<Option<[u8; 32]>, ServiceError> {
        let key = format!("zone_log/{}", self.zone_id);
        match self.store.get(key.as_bytes())? {
            Some(bytes) => {
                let cert: FinalityCertificate = grid_core::decode_canonical(&bytes)
                    .map_err(|e| ServiceError::Storage(e.to_string()))?;
                Ok(Some(cert.block_hash))
            }
            None => Ok(None),
        }
    }

    /// Get the full history of finality certificates for this zone.
    pub fn list_certificates(&self) -> Result<Vec<FinalityCertificate>, ServiceError> {
        let key = format!("zone_log/{}", self.zone_id);
        let entries = self.store.list(key.as_bytes())?;
        let mut certs = Vec::with_capacity(entries.len());
        for entry in &entries {
            let cert: FinalityCertificate = grid_core::decode_canonical(entry)
                .map_err(|e| ServiceError::Storage(e.to_string()))?;
            certs.push(cert);
        }
        Ok(certs)
    }

    /// Get the number of finalized blocks.
    pub fn len(&self) -> Result<u64, ServiceError> {
        let key = format!("zone_log/{}", self.zone_id);
        self.store.len(key.as_bytes())
    }

    pub fn is_empty(&self) -> Result<bool, ServiceError> {
        Ok(self.len()? == 0)
    }

    pub fn zone_id(&self) -> ZoneId {
        self.zone_id
    }
}
