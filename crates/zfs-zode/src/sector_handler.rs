use std::collections::HashSet;
use std::sync::Arc;

use tracing::{debug, warn};
use zfs_core::{
    ErrorCode, GossipSectorAppend, ProgramId, SectorAppendRequest, SectorAppendResponse,
    SectorAppendResult, SectorBatchAppendEntry, SectorBatchAppendRequest,
    SectorBatchAppendResponse, SectorBatchLogLengthRequest, SectorBatchLogLengthResponse,
    SectorLogLengthRequest, SectorLogLengthResponse, SectorLogLengthResult, SectorReadLogRequest,
    SectorReadLogResponse, SectorRequest, SectorResponse, MAX_BATCH_ENTRIES,
    MAX_BATCH_PAYLOAD_BYTES,
};
use zfs_storage::{SectorStore, StorageError};

use crate::config::{SectorFilter, SectorLimitsConfig};
use crate::metrics::ZodeMetrics;

/// Handles incoming sector protocol requests, enforcing policy and limits
/// before delegating to `SectorStore`.
pub(crate) struct SectorRequestHandler<S> {
    storage: Arc<S>,
    topics: HashSet<ProgramId>,
    limits: SectorLimitsConfig,
    sector_filter: SectorFilter,
    metrics: Arc<ZodeMetrics>,
}

impl<S: SectorStore> SectorRequestHandler<S> {
    pub(crate) fn new(
        storage: Arc<S>,
        topics: HashSet<ProgramId>,
        limits: SectorLimitsConfig,
        sector_filter: SectorFilter,
        metrics: Arc<ZodeMetrics>,
    ) -> Self {
        Self {
            storage,
            topics,
            limits,
            sector_filter,
            metrics,
        }
    }

    /// Dispatch a sector request to the appropriate handler.
    pub(crate) fn handle_sector_request(&self, req: &SectorRequest) -> SectorResponse {
        match req {
            SectorRequest::Append(r) => SectorResponse::Append(self.handle_append(r)),
            SectorRequest::ReadLog(r) => SectorResponse::ReadLog(self.handle_read_log(r)),
            SectorRequest::LogLength(r) => SectorResponse::LogLength(self.handle_log_length(r)),
            SectorRequest::BatchAppend(r) => {
                SectorResponse::BatchAppend(self.handle_batch_append(r))
            }
            SectorRequest::BatchLogLength(r) => {
                SectorResponse::BatchLogLength(self.handle_batch_log_length(r))
            }
        }
    }

    fn handle_append(&self, req: &SectorAppendRequest) -> SectorAppendResponse {
        if let Err(code) = self.check_access(&req.program_id, &req.sector_id) {
            return SectorAppendResponse {
                ok: false,
                index: None,
                error_code: Some(code),
            };
        }
        if let Err(code) = self.check_entry_size(req.entry.len()) {
            return SectorAppendResponse {
                ok: false,
                index: None,
                error_code: Some(code),
            };
        }
        match self
            .storage
            .append(&req.program_id, &req.sector_id, &req.entry)
        {
            Ok(index) => {
                self.metrics.inc_sectors_stored();
                SectorAppendResponse {
                    ok: true,
                    index: Some(index),
                    error_code: None,
                }
            }
            Err(e) => SectorAppendResponse {
                ok: false,
                index: None,
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    fn handle_read_log(&self, req: &SectorReadLogRequest) -> SectorReadLogResponse {
        if let Err(code) = self.check_access(&req.program_id, &req.sector_id) {
            return SectorReadLogResponse {
                entries: Vec::new(),
                error_code: Some(code),
            };
        }
        let max = (req.max_entries as usize).min(MAX_BATCH_ENTRIES);
        match self
            .storage
            .read_log(&req.program_id, &req.sector_id, req.from_index, max)
        {
            Ok(entries) => SectorReadLogResponse {
                entries: entries
                    .into_iter()
                    .map(serde_bytes::ByteBuf::from)
                    .collect(),
                error_code: None,
            },
            Err(e) => SectorReadLogResponse {
                entries: Vec::new(),
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    fn handle_log_length(&self, req: &SectorLogLengthRequest) -> SectorLogLengthResponse {
        if let Err(code) = self.check_access(&req.program_id, &req.sector_id) {
            return SectorLogLengthResponse {
                length: 0,
                error_code: Some(code),
            };
        }
        match self.storage.log_length(&req.program_id, &req.sector_id) {
            Ok(length) => SectorLogLengthResponse {
                length,
                error_code: None,
            },
            Err(e) => SectorLogLengthResponse {
                length: 0,
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    fn handle_batch_append(&self, req: &SectorBatchAppendRequest) -> SectorBatchAppendResponse {
        if let Err(code) = self.check_program(&req.program_id) {
            return SectorBatchAppendResponse {
                results: reject_all(&req.entries, code),
            };
        }
        if let Err(code) = self.check_batch_append_limits(&req.entries) {
            return SectorBatchAppendResponse {
                results: reject_all(&req.entries, code),
            };
        }
        let results = req
            .entries
            .iter()
            .map(|e| self.append_one(&req.program_id, e))
            .collect();
        SectorBatchAppendResponse { results }
    }

    fn append_one(&self, pid: &ProgramId, entry: &SectorBatchAppendEntry) -> SectorAppendResult {
        if !self.sector_allowed(&entry.sector_id) {
            return SectorAppendResult {
                ok: false,
                index: None,
                error_code: Some(ErrorCode::PolicyReject),
            };
        }
        match self.storage.append(pid, &entry.sector_id, &entry.entry) {
            Ok(index) => {
                self.metrics.inc_sectors_stored();
                SectorAppendResult {
                    ok: true,
                    index: Some(index),
                    error_code: None,
                }
            }
            Err(e) => SectorAppendResult {
                ok: false,
                index: None,
                error_code: Some(storage_err_to_code(&e)),
            },
        }
    }

    fn handle_batch_log_length(
        &self,
        req: &SectorBatchLogLengthRequest,
    ) -> SectorBatchLogLengthResponse {
        if let Err(code) = self.check_program(&req.program_id) {
            return SectorBatchLogLengthResponse {
                results: Vec::new(),
                error_code: Some(code),
            };
        }
        if req.sector_ids.len() > MAX_BATCH_ENTRIES {
            return SectorBatchLogLengthResponse {
                results: Vec::new(),
                error_code: Some(ErrorCode::BatchTooLarge),
            };
        }
        let results = req
            .sector_ids
            .iter()
            .map(|sid| match self.storage.log_length(&req.program_id, sid) {
                Ok(length) => SectorLogLengthResult {
                    length,
                    error_code: None,
                },
                Err(e) => SectorLogLengthResult {
                    length: 0,
                    error_code: Some(storage_err_to_code(&e)),
                },
            })
            .collect();
        SectorBatchLogLengthResponse {
            results,
            error_code: None,
        }
    }

    /// Handle a gossip sector append. Returns `true` if stored.
    pub(crate) fn handle_gossip_append(&self, msg: &GossipSectorAppend) -> bool {
        if self.check_access(&msg.program_id, &msg.sector_id).is_err() {
            return false;
        }
        if let Err(code) = self.check_entry_size(msg.payload.len()) {
            debug!(?code, "gossip append rejected by size limit");
            return false;
        }
        match self
            .storage
            .insert_at(&msg.program_id, &msg.sector_id, msg.index, &msg.payload)
        {
            Ok(stored) => {
                if stored {
                    self.metrics.inc_sectors_stored();
                }
                true
            }
            Err(e) => {
                warn!(error = %e, "gossip append store failed");
                false
            }
        }
    }

    fn check_access(
        &self,
        program_id: &ProgramId,
        sector_id: &zfs_core::SectorId,
    ) -> Result<(), ErrorCode> {
        self.check_program(program_id)?;
        if !self.sector_allowed(sector_id) {
            self.metrics.inc_policy_rejection();
            return Err(ErrorCode::PolicyReject);
        }
        Ok(())
    }

    fn check_program(&self, program_id: &ProgramId) -> Result<(), ErrorCode> {
        if self.topics.contains(program_id) {
            Ok(())
        } else {
            self.metrics.inc_policy_rejection();
            Err(ErrorCode::PolicyReject)
        }
    }

    fn sector_allowed(&self, sector_id: &zfs_core::SectorId) -> bool {
        match &self.sector_filter {
            SectorFilter::All => true,
            SectorFilter::AllowList(set) => set.contains(sector_id),
        }
    }

    fn check_entry_size(&self, size: usize) -> Result<(), ErrorCode> {
        if size as u64 > self.limits.max_slot_size_bytes {
            self.metrics.inc_limit_rejection();
            Err(ErrorCode::InvalidPayload)
        } else {
            Ok(())
        }
    }

    fn check_batch_append_limits(
        &self,
        entries: &[SectorBatchAppendEntry],
    ) -> Result<(), ErrorCode> {
        if entries.len() > MAX_BATCH_ENTRIES {
            return Err(ErrorCode::BatchTooLarge);
        }
        let total: usize = entries.iter().map(|e| e.entry.len()).sum();
        if total > MAX_BATCH_PAYLOAD_BYTES {
            return Err(ErrorCode::BatchTooLarge);
        }
        Ok(())
    }
}

fn reject_all(entries: &[SectorBatchAppendEntry], code: ErrorCode) -> Vec<SectorAppendResult> {
    entries
        .iter()
        .map(|_| SectorAppendResult {
            ok: false,
            index: None,
            error_code: Some(code),
        })
        .collect()
}

fn storage_err_to_code(e: &StorageError) -> ErrorCode {
    match e {
        StorageError::BatchTooLarge(_) => ErrorCode::BatchTooLarge,
        StorageError::Full { .. } => ErrorCode::StorageFull,
        _ => ErrorCode::InvalidPayload,
    }
}
