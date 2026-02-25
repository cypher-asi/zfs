use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zfs_core::{ProgramId, SectorId, ZfsError};

/// Maximum Interlink message size (64 KiB).
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;

const CHANNEL_SECTOR_PREFIX: &[u8] = b"zchat/channel/";
const MESSAGE_SECTOR_PREFIX: &[u8] = b"zchat/msg/";

/// Interlink program descriptor.
///
/// Defines the chat program parameters. The `program_id` is derived
/// as `SHA-256(canonical_cbor(self))`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZChatDescriptor {
    pub name: String,
    pub version: u32,
    pub proof_required: bool,
}

impl ZChatDescriptor {
    /// Create the canonical v1 Interlink descriptor.
    pub fn v1() -> Self {
        Self {
            name: "zchat".to_owned(),
            version: 1,
            proof_required: false,
        }
    }

    /// Derive the ProgramId from this descriptor.
    pub fn program_id(&self) -> Result<ProgramId, ZfsError> {
        let canonical = self.encode_canonical()?;
        Ok(ProgramId::from_descriptor_bytes(&canonical))
    }

    /// Build the GossipSub topic string.
    pub fn topic(&self) -> Result<String, ZfsError> {
        Ok(crate::program_topic(&self.program_id()?))
    }

    /// Encode to canonical CBOR bytes.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, ZfsError> {
        zfs_core::encode_canonical(self)
    }

    /// Decode from canonical CBOR bytes.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, ZfsError> {
        zfs_core::decode_canonical(bytes)
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
    /// `SectorId = SHA-256("zchat/channel/" || channel_id_bytes)`.
    pub fn sector_id(&self) -> SectorId {
        sector_id_for_channel(self)
    }
}

/// Derive a deterministic SectorId from a ChannelId.
///
/// Uses `SHA-256("zchat/channel/" || channel_id_bytes)` to produce a
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
/// `SHA-256("zchat/msg/" || channel_id || "/" || timestamp_ms_le || "/" || sender_did)`.
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZChatMessage {
    /// DID of the sender.
    pub sender_did: String,
    /// Channel this message belongs to.
    pub channel_id: ChannelId,
    /// Message content (UTF-8 text).
    pub content: String,
    /// Timestamp (milliseconds since epoch).
    pub timestamp_ms: u64,
}

impl ZChatMessage {
    /// Encode to canonical CBOR bytes, enforcing the size limit.
    pub fn encode_canonical(&self) -> Result<Vec<u8>, ZfsError> {
        let bytes = zfs_core::encode_canonical(self)?;
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(ZfsError::InvalidPayload(format!(
                "ZChatMessage exceeds max size: {} > {}",
                bytes.len(),
                MAX_MESSAGE_SIZE
            )));
        }
        Ok(bytes)
    }

    /// Decode from canonical CBOR bytes.
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, ZfsError> {
        zfs_core::decode_canonical(bytes)
    }
}

/// Reserved test channel ID for zode-app test traffic.
pub const TEST_CHANNEL_ID: &str = "INTERLINK-MAIN";
