use serde::{Deserialize, Serialize};

use crate::{ErrorCode, ProgramId, SectorId};

/// Maximum entries in a single batch request.
pub const MAX_BATCH_ENTRIES: usize = 64;

/// Maximum total payload bytes in a single batch request (4 MB).
pub const MAX_BATCH_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Client → Zode: sector request sent over `/zfs/sector/1.0.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SectorRequest {
    Store(SectorStoreRequest),
    Fetch(SectorFetchRequest),
    BatchStore(SectorBatchStoreRequest),
    BatchFetch(SectorBatchFetchRequest),
}

/// Zode → Client: sector response sent over `/zfs/sector/1.0.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SectorResponse {
    Store(SectorStoreResponse),
    Fetch(SectorFetchResponse),
    BatchStore(SectorBatchStoreResponse),
    BatchFetch(SectorBatchFetchResponse),
}

/// Store a single sector payload (write-once or mutable with optional CAS).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorStoreRequest {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
    /// Allow overwriting an existing value. `false` = write-once.
    pub overwrite: bool,
    /// Optional SHA-256 hash of the current payload for compare-and-swap.
    /// Only checked when `overwrite` is true.
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::serde_helpers::opt_bytes"
    )]
    pub expected_hash: Option<Vec<u8>>,
}

/// Response to a single sector store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorStoreResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

/// Fetch a single sector by ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorFetchRequest {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
}

/// Response to a single sector fetch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorFetchResponse {
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::serde_helpers::opt_bytes"
    )]
    pub payload: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

/// One entry in a batch store request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchStoreEntry {
    pub sector_id: SectorId,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
    pub overwrite: bool,
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::serde_helpers::opt_bytes"
    )]
    pub expected_hash: Option<Vec<u8>>,
}

/// Batch store request (all entries share one program_id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchStoreRequest {
    pub program_id: ProgramId,
    pub entries: Vec<SectorBatchStoreEntry>,
}

/// Per-entry result in a batch store response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorStoreResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

/// Response to a batch store request (one result per entry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchStoreResponse {
    pub results: Vec<SectorStoreResult>,
}

/// Batch fetch request (all entries share one program_id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchFetchRequest {
    pub program_id: ProgramId,
    pub sector_ids: Vec<SectorId>,
}

/// Response to a batch fetch (one optional payload per requested ID).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchFetchResponse {
    pub payloads: Vec<Option<serde_bytes::ByteBuf>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

/// Lightweight sector announcement for GossipSub propagation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GossipSector {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
    pub overwrite: bool,
}
