use grid_core::{CborType, FieldDef, FieldSchema, GridError, ProgramId, ProofSystem, SectorId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Maximum Interlink message size (64 KiB).
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;

const CHANNEL_SECTOR_PREFIX: &[u8] = b"interlink/channel/";
const MESSAGE_SECTOR_PREFIX: &[u8] = b"interlink/msg/";

/// Interlink program descriptor.
///
/// Defines the chat program parameters. The `program_id` is derived
/// as `SHA-256(canonical_cbor(self))`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterlinkDescriptor {
    pub name: String,
    pub version: u32,
    pub proof_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_system: Option<ProofSystem>,
}

impl InterlinkDescriptor {
    /// Create the canonical v1 Interlink descriptor.
    pub fn v1() -> Self {
        Self {
            name: "interlink".to_owned(),
            version: 1,
            proof_required: false,
            proof_system: None,
        }
    }

    /// Create the v2 descriptor with Groth16 shape proofs.
    pub fn v2() -> Self {
        Self {
            name: "interlink".to_owned(),
            version: 2,
            proof_required: true,
            proof_system: Some(ProofSystem::Groth16),
        }
    }

    /// Canonical field schema for Interlink messages (v2+).
    pub fn field_schema() -> FieldSchema {
        FieldSchema {
            program_name: "interlink".into(),
            version: 1,
            fields: vec![
                FieldDef {
                    key: "channel_id".into(),
                    value_type: CborType::ByteString,
                    optional: false,
                },
                FieldDef {
                    key: "content".into(),
                    value_type: CborType::TextString,
                    optional: false,
                },
                FieldDef {
                    key: "sender_did".into(),
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
        grid_core::decode_canonical(bytes)
    }
}

/// Logical channel identifier for Interlink.
///
/// Chat messages use per-message sectors via [`sector_id_for_message`].
/// The legacy per-channel sector ID is retained for non-chat uses.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelId(#[serde(with = "serde_bytes")] Vec<u8>);

impl ChannelId {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn from_str_id(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Derive the SectorId for this channel.
    ///
    /// `SectorId = SHA-256("interlink/channel/" || channel_id_bytes)`.
    pub fn sector_id(&self) -> SectorId {
        sector_id_for_channel(self)
    }
}

/// Derive a deterministic SectorId from a ChannelId.
///
/// Uses `SHA-256("interlink/channel/" || channel_id_bytes)` to produce a
/// fixed-size sector identifier.
pub fn sector_id_for_channel(channel_id: &ChannelId) -> SectorId {
    let mut hasher = Sha256::new();
    hasher.update(CHANNEL_SECTOR_PREFIX);
    hasher.update(channel_id.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    SectorId::from_bytes(hash.to_vec())
}

/// Derive a unique write-once SectorId for a single chat message.
///
/// `SHA-256("interlink/msg/" || channel_id || "/" || timestamp_ms_le || "/" || sender_did)`.
/// Practically collision-free: unique per sender per millisecond per channel.
pub fn sector_id_for_message(
    channel_id: &ChannelId,
    timestamp_ms: u64,
    sender_did: &str,
) -> SectorId {
    let mut hasher = Sha256::new();
    hasher.update(MESSAGE_SECTOR_PREFIX);
    hasher.update(channel_id.as_bytes());
    hasher.update(b"/");
    hasher.update(timestamp_ms.to_le_bytes());
    hasher.update(b"/");
    hasher.update(sender_did.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    SectorId::from_bytes(hash.to_vec())
}

/// An Interlink message.
///
/// Size limit: [`MAX_MESSAGE_SIZE`] (64 KiB) for the canonical CBOR encoding.
/// The `signature` field carries a PQ-hybrid signature (Ed25519 + ML-DSA-65)
/// produced by the sender's `IdentitySigningKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZMessage {
    /// DID of the sender.
    pub sender_did: String,
    /// Channel this message belongs to.
    pub channel_id: ChannelId,
    /// Message content (UTF-8 text).
    pub content: String,
    /// Timestamp (milliseconds since epoch).
    pub timestamp_ms: u64,
    /// PQ-hybrid signature: `HybridSignature::to_bytes()` (Ed25519 64 || ML-DSA-65 3309).
    #[serde(with = "serde_bytes", default)]
    pub signature: Vec<u8>,
}

impl ZMessage {
    /// Build a signed message.
    ///
    /// `sign_fn` receives the canonical signable bytes and must return the
    /// serialised signature (e.g. `HybridSignature::to_bytes()`).
    pub fn new_signed(
        sender_did: String,
        channel_id: ChannelId,
        content: String,
        timestamp_ms: u64,
        sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<Self, GridError> {
        let mut msg = Self {
            sender_did,
            channel_id,
            content,
            timestamp_ms,
            signature: Vec::new(),
        };
        let signable = msg.signable_bytes()?;
        msg.signature = sign_fn(&signable);
        Ok(msg)
    }

    /// Verify the embedded signature against [`signable_bytes()`](Self::signable_bytes).
    ///
    /// `verify_fn` receives `(signable_bytes, signature_bytes)` and returns
    /// `true` when the signature is valid.  Returns `Ok(false)` when the
    /// signature field is empty (unsigned message).
    pub fn verify_signature(
        &self,
        verify_fn: impl FnOnce(&[u8], &[u8]) -> bool,
    ) -> Result<bool, GridError> {
        if self.signature.is_empty() {
            return Ok(false);
        }
        let signable = self.signable_bytes()?;
        Ok(verify_fn(&signable, &self.signature))
    }

    /// Canonical CBOR of all fields EXCEPT `signature`.
    /// This is the payload that gets signed and later verified.
    pub fn signable_bytes(&self) -> Result<Vec<u8>, GridError> {
        #[derive(Serialize)]
        struct Signable<'a> {
            sender_did: &'a str,
            channel_id: &'a ChannelId,
            content: &'a str,
            timestamp_ms: u64,
        }
        grid_core::encode_canonical(&Signable {
            sender_did: &self.sender_did,
            channel_id: &self.channel_id,
            content: &self.content,
            timestamp_ms: self.timestamp_ms,
        })
    }

    /// Encode to canonical CBOR bytes, enforcing the size limit.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, GridError> {
        let bytes = grid_core::encode_canonical(self)?;
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(GridError::InvalidPayload(format!(
                "ZMessage exceeds max size: {} > {}",
                bytes.len(),
                MAX_MESSAGE_SIZE
            )));
        }
        Ok(bytes)
    }

    /// Decode from canonical CBOR bytes.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, GridError> {
        grid_core::decode_canonical(bytes)
    }
}

/// Reserved test channel ID for zode-bin test traffic.
pub const TEST_CHANNEL_ID: &str = "INTERLINK-MAIN";
