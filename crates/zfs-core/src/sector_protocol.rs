use serde::{Deserialize, Serialize};

use crate::{ErrorCode, ProgramId, SectorId};

/// Maximum entries in a single batch request.
pub const MAX_BATCH_ENTRIES: usize = 64;

/// Maximum total payload bytes in a single batch request (4 MB).
pub const MAX_BATCH_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Top-level request / response enums
// ---------------------------------------------------------------------------

/// Client → Zode: sector request sent over `/zfs/sector/2.0.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SectorRequest {
    Append(SectorAppendRequest),
    ReadLog(SectorReadLogRequest),
    LogLength(SectorLogLengthRequest),
    BatchAppend(SectorBatchAppendRequest),
    BatchLogLength(SectorBatchLogLengthRequest),
}

/// Zode → Client: sector response sent over `/zfs/sector/2.0.0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SectorResponse {
    Append(SectorAppendResponse),
    ReadLog(SectorReadLogResponse),
    LogLength(SectorLogLengthResponse),
    BatchAppend(SectorBatchAppendResponse),
    BatchLogLength(SectorBatchLogLengthResponse),
}

// ---------------------------------------------------------------------------
// Append
// ---------------------------------------------------------------------------

/// Append a single entry to a sector log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorAppendRequest {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
    #[serde(with = "serde_bytes")]
    pub entry: Vec<u8>,
}

/// Response to a sector append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorAppendResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

// ---------------------------------------------------------------------------
// ReadLog
// ---------------------------------------------------------------------------

/// Read entries from a sector log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorReadLogRequest {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
    pub from_index: u64,
    pub max_entries: u32,
}

/// Response to a sector read-log request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorReadLogResponse {
    pub entries: Vec<serde_bytes::ByteBuf>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

// ---------------------------------------------------------------------------
// LogLength
// ---------------------------------------------------------------------------

/// Query the length of a sector log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorLogLengthRequest {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
}

/// Response to a sector log-length query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorLogLengthResponse {
    pub length: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

// ---------------------------------------------------------------------------
// BatchAppend
// ---------------------------------------------------------------------------

/// One entry in a batch append request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchAppendEntry {
    pub sector_id: SectorId,
    #[serde(with = "serde_bytes")]
    pub entry: Vec<u8>,
}

/// Batch append: multiple entries to different sectors under one program.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchAppendRequest {
    pub program_id: ProgramId,
    pub entries: Vec<SectorBatchAppendEntry>,
}

/// Per-entry result in a batch append response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorAppendResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

/// Response to a batch append.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchAppendResponse {
    pub results: Vec<SectorAppendResult>,
}

// ---------------------------------------------------------------------------
// BatchLogLength
// ---------------------------------------------------------------------------

/// Batch log-length query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchLogLengthRequest {
    pub program_id: ProgramId,
    pub sector_ids: Vec<SectorId>,
}

/// Per-sector result in a batch log-length response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectorLogLengthResult {
    pub length: u64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

/// Response to a batch log-length query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorBatchLogLengthResponse {
    pub results: Vec<SectorLogLengthResult>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error_code: Option<ErrorCode>,
}

// ---------------------------------------------------------------------------
// Gossip
// ---------------------------------------------------------------------------

/// Lightweight sector append announcement for GossipSub propagation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GossipSectorAppend {
    pub program_id: ProgramId,
    pub sector_id: SectorId,
    pub index: u64,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
}
