use serde::{Deserialize, Serialize};

/// A direct message between two Zodes, tagged with a topic for routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectMessage {
    pub topic: String,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
}

/// Acknowledgement for a direct message (fire-and-forget semantics).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectMessageAck {
    pub ok: bool,
}
