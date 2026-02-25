use std::collections::HashSet;
use std::sync::Arc;

use tracing::{debug, warn};
use zfs_core::{
    ErrorCode, GossipSector, ProgramId, SectorBatchFetchRequest, SectorBatchFetchResponse,
    SectorBatchStoreEntry, SectorBatchStoreRequest, SectorBatchStoreResponse, SectorFetchRequest,
    SectorFetchResponse, SectorRequest, SectorResponse, SectorStoreRequest, SectorStoreResponse,
    SectorStoreResult, MAX_BATCH_ENTRIES, MAX_BATCH_PAYLOAD_BYTES,
};
use zfs_storage::{SectorStore, StorageError};

use crate::config::SectorLimitsConfig;
use crate::metrics::ZodeMetrics;

/// Handles incoming sector protocol requests, enforcing policy and limits
/// before delegating to `SectorStore`.
pub(crate) struct SectorRequestHandler<S> {
    storage: Arc<S>,
    topics: HashSet<ProgramId>,
    limits: SectorLimitsConfig,
    metrics: Arc<ZodeMetrics>,
}

impl<S: SectorStore> SectorRequestHandler<S> {
    pub(crate) fn new(
        storage: Arc<S>,
        topics: HashSet<ProgramId>,
        limits: SectorLimitsConfig,
        metrics: Arc<ZodeMetrics>,
    ) -> Self {
        Self {
            storage,
            topics,
            limits,
            metrics,
        }
    }

    /// Dispatch a sector request to the appropriate handler.
    pub(crate) fn handle_sector_request(&self, req: &SectorRequest) -> SectorResponse {
        match req {
            SectorRequest::Store(r) => SectorResponse::Store(self.handle_store(r)),
            SectorRequest::Fetch(r) => SectorResponse::Fetch(self.handle_fetch(r)),
            SectorRequest::BatchStore(r) => SectorResponse::BatchStore(self.handle_batch_store(r)),
            SectorRequest::BatchFetch(r) => SectorResponse::BatchFetch(self.handle_batch_fetch(r)),
        }
    }

    fn handle_store(&self, req: &SectorStoreRequest) -> SectorStoreResponse {
        if let Err(code) = self.check_program(&req.program_id) {
            return SectorStoreResponse {
                ok: false,
                error_code: Some(code),
            };
        }
        if let Err(code) = self.check_slot_size(req.payload.len()) {
            return SectorStoreResponse {
                ok: false,
                error_code: Some(code),
            };
        }

        match self.storage.put(
            &req.program_id,
            &req.sector_id,
            &req.payload,
            req.overwrite,
            req.expected_hash.as_deref(),
        ) {
            Ok(()) => {
                self.metrics.inc_sectors_stored();
                SectorStoreResponse {
                    ok: true,
                    error_code: None,
                }
            }
            Err(e) => SectorStoreResponse {
                ok: false,
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    fn handle_fetch(&self, req: &SectorFetchRequest) -> SectorFetchResponse {
        if let Err(code) = self.check_program(&req.program_id) {
            return SectorFetchResponse {
                payload: None,
                error_code: Some(code),
            };
        }
        match self.storage.get(&req.program_id, &req.sector_id) {
            Ok(payload) => SectorFetchResponse {
                payload,
                error_code: None,
            },
            Err(e) => SectorFetchResponse {
                payload: None,
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    fn handle_batch_store(&self, req: &SectorBatchStoreRequest) -> SectorBatchStoreResponse {
        if let Err(code) = self.check_program(&req.program_id) {
            let results = req
                .entries
                .iter()
                .map(|_| SectorStoreResult {
                    ok: false,
                    error_code: Some(code),
                })
                .collect();
            return SectorBatchStoreResponse { results };
        }
        if let Err(code) = self.check_batch_limits(&req.entries) {
            let results = req
                .entries
                .iter()
                .map(|_| SectorStoreResult {
                    ok: false,
                    error_code: Some(code),
                })
                .collect();
            return SectorBatchStoreResponse { results };
        }

        let entries: Vec<_> = req
            .entries
            .iter()
            .map(|e| {
                (
                    e.sector_id.clone(),
                    e.payload.clone(),
                    e.overwrite,
                    e.expected_hash.clone(),
                )
            })
            .collect();

        match self.storage.batch_put(&req.program_id, &entries) {
            Ok(put_results) => {
                let results = put_results
                    .into_iter()
                    .map(|r| {
                        if r.ok {
                            self.metrics.inc_sectors_stored();
                        }
                        SectorStoreResult {
                            ok: r.ok,
                            error_code: r.error.as_ref().map(storage_err_to_code),
                        }
                    })
                    .collect();
                SectorBatchStoreResponse { results }
            }
            Err(e) => {
                let code = storage_err_to_code(&e);
                let results = req
                    .entries
                    .iter()
                    .map(|_| SectorStoreResult {
                        ok: false,
                        error_code: Some(code),
                    })
                    .collect();
                SectorBatchStoreResponse { results }
            }
        }
    }

    fn handle_batch_fetch(&self, req: &SectorBatchFetchRequest) -> SectorBatchFetchResponse {
        if let Err(code) = self.check_program(&req.program_id) {
            return SectorBatchFetchResponse {
                payloads: Vec::new(),
                error_code: Some(code),
            };
        }
        if req.sector_ids.len() > MAX_BATCH_ENTRIES {
            return SectorBatchFetchResponse {
                payloads: Vec::new(),
                error_code: Some(ErrorCode::BatchTooLarge),
            };
        }
        match self.storage.batch_get(&req.program_id, &req.sector_ids) {
            Ok(payloads) => {
                let payloads = payloads
                    .into_iter()
                    .map(|opt| opt.map(serde_bytes::ByteBuf::from))
                    .collect();
                SectorBatchFetchResponse {
                    payloads,
                    error_code: None,
                }
            }
            Err(e) => SectorBatchFetchResponse {
                payloads: Vec::new(),
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    /// Handle a gossip sector announcement. Returns `true` if stored.
    pub(crate) fn handle_gossip_sector(&self, msg: &GossipSector) -> bool {
        if self.check_program(&msg.program_id).is_err() {
            return false;
        }
        if let Err(code) = self.check_slot_size(msg.payload.len()) {
            debug!(?code, "gossip sector rejected by size limit");
            return false;
        }

        let overwrite = msg.overwrite;
        match self.storage.put(
            &msg.program_id,
            &msg.sector_id,
            &msg.payload,
            overwrite,
            None,
        ) {
            Ok(()) => {
                self.metrics.inc_sectors_stored();
                true
            }
            Err(StorageError::SlotOccupied) if !overwrite => true,
            Err(e) => {
                warn!(error = %e, "gossip sector store failed");
                false
            }
        }
    }

    fn check_program(&self, program_id: &ProgramId) -> Result<(), ErrorCode> {
        if self.topics.contains(program_id) {
            Ok(())
        } else {
            self.metrics.inc_policy_rejection();
            Err(ErrorCode::PolicyReject)
        }
    }

    fn check_slot_size(&self, size: usize) -> Result<(), ErrorCode> {
        if size as u64 > self.limits.max_slot_size_bytes {
            self.metrics.inc_limit_rejection();
            Err(ErrorCode::InvalidPayload)
        } else {
            Ok(())
        }
    }

    fn check_batch_limits(&self, entries: &[SectorBatchStoreEntry]) -> Result<(), ErrorCode> {
        if entries.len() > MAX_BATCH_ENTRIES {
            return Err(ErrorCode::BatchTooLarge);
        }
        let total: usize = entries.iter().map(|e| e.payload.len()).sum();
        if total > MAX_BATCH_PAYLOAD_BYTES {
            return Err(ErrorCode::BatchTooLarge);
        }
        Ok(())
    }
}

fn storage_err_to_code(e: &StorageError) -> ErrorCode {
    match e {
        StorageError::SlotOccupied => ErrorCode::SlotOccupied,
        StorageError::ConditionFailed => ErrorCode::ConditionFailed,
        StorageError::BatchTooLarge(_) => ErrorCode::BatchTooLarge,
        StorageError::Full { .. } => ErrorCode::StorageFull,
        _ => ErrorCode::InvalidPayload,
    }
}
