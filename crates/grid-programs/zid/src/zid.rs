use grid_core::{CborType, FieldDef, FieldSchema, GridError, ProgramId, ProofSystem};
use serde::{Deserialize, Serialize};

/// Maximum input size for ZID decode operations (64 KiB).
const MAX_INPUT_SIZE: usize = 64 * 1024;

/// ZID (Zero Identity) program descriptor.
///
/// Defines the identity program parameters. The `program_id` is derived
/// as `SHA-256(canonical_cbor(self))`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZidDescriptor {
    pub name: String,
    pub version: u32,
    pub proof_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_system: Option<ProofSystem>,
}

impl ZidDescriptor {
    /// Create the canonical v1 ZID descriptor.
    pub fn v1() -> Self {
        Self {
            name: "zid".to_owned(),
            version: 1,
            proof_required: false,
            proof_system: None,
        }
    }

    /// Create the v2 descriptor with Groth16 shape proofs.
    pub fn v2() -> Self {
        Self {
            name: "zid".to_owned(),
            version: 2,
            proof_required: true,
            proof_system: Some(ProofSystem::Groth16),
        }
    }

    /// Canonical field schema for ZID messages (v2+).
    pub fn field_schema() -> FieldSchema {
        FieldSchema {
            program_name: "zid".into(),
            version: 1,
            fields: vec![
                FieldDef {
                    key: "display_name".into(),
                    value_type: CborType::TextString,
                    optional: true,
                },
                FieldDef {
                    key: "owner_did".into(),
                    value_type: CborType::TextString,
                    optional: false,
                },
                FieldDef {
                    key: "signature".into(),
                    value_type: CborType::ByteString,
                    optional: false,
                },
                FieldDef {
                    key: "timestamp_ms".into(),
                    value_type: CborType::UnsignedInt,
                    optional: false,
                },
            ],
        }
    }

    /// Derive the ProgramId from this descriptor.
    pub fn program_id(&self) -> Result<ProgramId, GridError> {
        let canonical = self.encode_canonical()?;
        Ok(ProgramId::from_descriptor_bytes(&canonical))
    }

    /// Build the GossipSub topic string.
    pub fn topic(&self) -> Result<String, GridError> {
        Ok(grid_core::program_topic(&self.program_id()?))
    }

    /// Encode to canonical CBOR bytes.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, GridError> {
        grid_core::encode_canonical(self)
    }

    /// Decode from canonical CBOR bytes.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, GridError> {
        if bytes.len() > MAX_INPUT_SIZE {
            return Err(GridError::InvalidPayload(format!(
                "ZidDescriptor input too large: {} > {MAX_INPUT_SIZE}",
                bytes.len(),
            )));
        }
        grid_core::decode_canonical(bytes)
    }
}

/// A ZID identity message (claim or update).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZidMessage {
    /// DID of the identity owner.
    pub owner_did: String,
    /// Display name (optional).
    pub display_name: Option<String>,
    /// Timestamp of the claim (milliseconds since epoch).
    pub timestamp_ms: u64,
    /// PQ-hybrid signature: `HybridSignature::to_bytes()`.
    #[serde(with = "serde_bytes", default)]
    pub signature: Vec<u8>,
}

impl ZidMessage {
    /// Canonical CBOR of all fields EXCEPT `signature`.
    pub fn signable_bytes(&self) -> Result<Vec<u8>, GridError> {
        #[derive(Serialize)]
        struct Signable<'a> {
            owner_did: &'a str,
            display_name: &'a Option<String>,
            timestamp_ms: u64,
        }
        grid_core::encode_canonical(&Signable {
            owner_did: &self.owner_did,
            display_name: &self.display_name,
            timestamp_ms: self.timestamp_ms,
        })
    }

    /// Encode to canonical CBOR bytes.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, GridError> {
        grid_core::encode_canonical(self)
    }

    /// Decode from canonical CBOR bytes.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, GridError> {
        if bytes.len() > MAX_INPUT_SIZE {
            return Err(GridError::InvalidPayload(format!(
                "ZidMessage input too large: {} > {MAX_INPUT_SIZE}",
                bytes.len(),
            )));
        }
        grid_core::decode_canonical(bytes)
    }
}
