use serde::{Deserialize, Serialize};

use crate::{ErrorCode, ProgramId, ProofSystem, SectorId};

/// Maximum entries in a single batch request.
pub const MAX_BATCH_ENTRIES: usize = 64;

/// Maximum total payload bytes in a single batch request (4 MB).
pub const MAX_BATCH_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Shape proof — binds ZK proof to stored ciphertext
// ---------------------------------------------------------------------------

/// A shape proof attesting that encrypted sector content conforms to a
/// program's field schema. Carries the Groth16 proof bytes and the
/// public inputs needed for verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeProof {
    pub proof_system: ProofSystem,
    /// `Poseidon(ciphertext)` — 32 bytes. The binding anchor: the Zode
    /// independently hashes the received ciphertext and checks equality.
    #[serde(with = "serde_bytes")]
    pub ciphertext_hash: Vec<u8>,
    /// Groth16 proof bytes (128 bytes on BN254).
    #[serde(with = "serde_bytes")]
    pub proof_bytes: Vec<u8>,
    /// `FieldSchema::schema_hash()` — 32 bytes.
    #[serde(with = "serde_bytes")]
    pub schema_hash: Vec<u8>,
    /// Which circuit size bucket was used (1024, 4096, …).
    pub size_bucket: u32,
}

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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub shape_proof: Option<ShapeProof>,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub shape_proof: Option<ShapeProof>,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub shape_proof: Option<ShapeProof>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{encode_canonical, decode_canonical};

    fn sample_shape_proof() -> ShapeProof {
        ShapeProof {
            proof_system: ProofSystem::Groth16,
            ciphertext_hash: vec![0xAA; 32],
            proof_bytes: vec![0xBB; 128],
            schema_hash: vec![0xCC; 32],
            size_bucket: 4096,
        }
    }

    #[test]
    fn shape_proof_serialization_round_trip() {
        let proof = sample_shape_proof();
        let bytes = encode_canonical(&proof).expect("encode");
        let decoded: ShapeProof = decode_canonical(&bytes).expect("decode");
        assert_eq!(proof, decoded);
    }

    #[test]
    fn sector_append_request_with_proof_round_trip() {
        let req = SectorAppendRequest {
            program_id: ProgramId::from([1u8; 32]),
            sector_id: SectorId::from_bytes(vec![2u8; 16]),
            entry: b"payload".to_vec(),
            shape_proof: Some(sample_shape_proof()),
        };
        let bytes = encode_canonical(&req).expect("encode");
        let decoded: SectorAppendRequest = decode_canonical(&bytes).expect("decode");
        assert_eq!(req, decoded);
        assert!(decoded.shape_proof.is_some());
    }

    #[test]
    fn sector_append_request_without_proof_round_trip() {
        let req = SectorAppendRequest {
            program_id: ProgramId::from([3u8; 32]),
            sector_id: SectorId::from_bytes(vec![4u8; 16]),
            entry: b"no proof".to_vec(),
            shape_proof: None,
        };
        let bytes = encode_canonical(&req).expect("encode");
        let decoded: SectorAppendRequest = decode_canonical(&bytes).expect("decode");
        assert_eq!(req, decoded);
        assert!(decoded.shape_proof.is_none());
    }

    #[test]
    fn gossip_with_proof_round_trip() {
        let gossip = GossipSectorAppend {
            program_id: ProgramId::from([5u8; 32]),
            sector_id: SectorId::from_bytes(vec![6u8; 16]),
            index: 42,
            payload: b"gossip payload".to_vec(),
            shape_proof: Some(sample_shape_proof()),
        };
        let bytes = encode_canonical(&gossip).expect("encode");
        let decoded: GossipSectorAppend = decode_canonical(&bytes).expect("decode");
        assert_eq!(gossip, decoded);
        assert!(decoded.shape_proof.is_some());
    }
}
