use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tracing::warn;
use zfs_core::{
    Cid, ErrorCode, GossipSectorAppend, ProgramId, ProofSystem, SectorAppendRequest,
    SectorAppendResponse, SectorAppendResult, SectorBatchAppendEntry, SectorBatchAppendRequest,
    SectorBatchAppendResponse, SectorBatchLogLengthRequest, SectorBatchLogLengthResponse,
    SectorLogLengthRequest, SectorLogLengthResponse, SectorLogLengthResult, SectorReadLogRequest,
    SectorReadLogResponse, SectorRequest, SectorResponse, ShapeProof, MAX_BATCH_ENTRIES,
    MAX_BATCH_PAYLOAD_BYTES,
};
use zfs_proof::ProofVerifierRegistry;
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
    proof_registry: Arc<ProofVerifierRegistry>,
    program_proof_config: HashMap<ProgramId, ProofSystem>,
}

impl<S: SectorStore> SectorRequestHandler<S> {
    pub(crate) fn new(
        storage: Arc<S>,
        topics: HashSet<ProgramId>,
        limits: SectorLimitsConfig,
        sector_filter: SectorFilter,
        metrics: Arc<ZodeMetrics>,
        proof_registry: Arc<ProofVerifierRegistry>,
        program_proof_config: HashMap<ProgramId, ProofSystem>,
    ) -> Self {
        Self {
            storage,
            topics,
            limits,
            sector_filter,
            metrics,
            proof_registry,
            program_proof_config,
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
        if let Err(code) = self.verify_proof(&req.program_id, &req.entry, req.shape_proof.as_ref())
        {
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
                if let Some(ref proof) = req.shape_proof {
                    let mut buf = Vec::new();
                    if ciborium::into_writer(proof, &mut buf).is_ok() {
                        let _ = self.storage.store_proof(
                            &req.program_id,
                            &req.sector_id,
                            index,
                            &buf,
                        );
                    }
                }
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
        if let Err(code) = self.verify_proof(pid, &entry.entry, entry.shape_proof.as_ref()) {
            return SectorAppendResult {
                ok: false,
                index: None,
                error_code: Some(code),
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

    /// Handle a gossip sector append, returning a detailed result.
    pub(crate) fn handle_gossip_append(
        &self,
        msg: &GossipSectorAppend,
    ) -> crate::types::GossipAppendResult {
        use crate::types::{GossipAppendResult, GossipRejectReason};

        if let Err(reason) = self.check_gossip_access(&msg.program_id, &msg.sector_id) {
            return GossipAppendResult::Rejected(reason);
        }
        if msg.payload.len() as u64 > self.limits.max_slot_size_bytes {
            self.metrics.inc_limit_rejection();
            return GossipAppendResult::Rejected(GossipRejectReason::EntryTooLarge {
                size: msg.payload.len(),
                max: self.limits.max_slot_size_bytes,
            });
        }
        if let Err(reason) =
            self.verify_proof_detailed(&msg.program_id, &msg.payload, msg.shape_proof.as_ref())
        {
            return GossipAppendResult::Rejected(reason);
        }
        match self
            .storage
            .insert_at(&msg.program_id, &msg.sector_id, msg.index, &msg.payload)
        {
            Ok(true) => {
                self.metrics.inc_sectors_stored();
                self.store_proof_if_present(msg);
                GossipAppendResult::Stored
            }
            Ok(false) => {
                self.handle_index_conflict(msg)
            }
            Err(e) => {
                warn!(error = %e, "gossip append store failed");
                GossipAppendResult::Rejected(GossipRejectReason::StorageError {
                    detail: e.to_string(),
                })
            }
        }
    }

    /// When `insert_at` returns false, check whether the existing entry at
    /// that index is identical (true duplicate from gossip retry) or a
    /// different message (multi-writer index conflict). On conflict, fall
    /// back to `append()` so the message is not lost.
    fn handle_index_conflict(
        &self,
        msg: &GossipSectorAppend,
    ) -> crate::types::GossipAppendResult {
        use crate::types::{GossipAppendResult, GossipRejectReason};

        let existing = self
            .storage
            .get_entry(&msg.program_id, &msg.sector_id, msg.index);

        match existing {
            Ok(Some(ref data)) if data == &msg.payload => GossipAppendResult::Duplicate,
            _ => {
                match self
                    .storage
                    .append(&msg.program_id, &msg.sector_id, &msg.payload)
                {
                    Ok(_new_index) => {
                        self.metrics.inc_sectors_stored();
                        self.store_proof_if_present(msg);
                        GossipAppendResult::Stored
                    }
                    Err(e) => {
                        warn!(error = %e, "gossip append (conflict fallback) failed");
                        GossipAppendResult::Rejected(GossipRejectReason::StorageError {
                            detail: e.to_string(),
                        })
                    }
                }
            }
        }
    }

    fn store_proof_if_present(&self, msg: &GossipSectorAppend) {
        if let Some(ref proof) = msg.shape_proof {
            let mut buf = Vec::new();
            if ciborium::into_writer(proof, &mut buf).is_ok() {
                let _ = self.storage.store_proof(
                    &msg.program_id,
                    &msg.sector_id,
                    msg.index,
                    &buf,
                );
            }
        }
    }

    /// Verify the shape proof for a sector append, if required.
    fn verify_proof(
        &self,
        program_id: &ProgramId,
        entry: &[u8],
        proof: Option<&ShapeProof>,
    ) -> Result<(), ErrorCode> {
        self.verify_proof_detailed(program_id, entry, proof)
            .map_err(|_| ErrorCode::ProofInvalid)
    }

    /// Like [`verify_proof`] but returns a specific [`GossipRejectReason`]
    /// for diagnostics.
    fn verify_proof_detailed(
        &self,
        program_id: &ProgramId,
        entry: &[u8],
        proof: Option<&ShapeProof>,
    ) -> Result<(), crate::types::GossipRejectReason> {
        use crate::types::GossipRejectReason;

        let proof_system = match self.program_proof_config.get(program_id) {
            Some(ps) => ps,
            None => return Ok(()),
        };
        if *proof_system == ProofSystem::None {
            return Ok(());
        }

        let shape_proof = proof.ok_or(GossipRejectReason::ProofMissing)?;

        let actual_ct_hash = zfs_crypto::poseidon_ciphertext_hash(entry)
            .map_err(|_| GossipRejectReason::CiphertextMalformed)?;

        if actual_ct_hash.as_slice() != shape_proof.ciphertext_hash.as_slice() {
            return Err(GossipRejectReason::CiphertextHashMismatch);
        }

        let mut payload_ctx = Vec::with_capacity(68);
        payload_ctx.extend_from_slice(&shape_proof.ciphertext_hash);
        payload_ctx.extend_from_slice(&shape_proof.schema_hash);
        payload_ctx.extend_from_slice(&shape_proof.size_bucket.to_le_bytes());

        let cid = Cid::from_ciphertext(entry);
        self.proof_registry
            .verify(
                &shape_proof.proof_system,
                &cid,
                program_id,
                0,
                &shape_proof.proof_bytes,
                Some(&payload_ctx),
            )
            .map_err(|e| GossipRejectReason::ProofVerificationFailed {
                detail: e.to_string(),
            })?;

        Ok(())
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

    /// Like [`check_access`] but returns [`GossipRejectReason`] for gossip diagnostics.
    fn check_gossip_access(
        &self,
        program_id: &ProgramId,
        sector_id: &zfs_core::SectorId,
    ) -> Result<(), crate::types::GossipRejectReason> {
        use crate::types::GossipRejectReason;
        if !self.topics.contains(program_id) {
            self.metrics.inc_policy_rejection();
            return Err(GossipRejectReason::ProgramNotSubscribed);
        }
        if !self.sector_allowed(sector_id) {
            self.metrics.inc_policy_rejection();
            return Err(GossipRejectReason::SectorFiltered);
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
