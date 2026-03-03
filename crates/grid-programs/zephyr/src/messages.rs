use serde::{Deserialize, Serialize};

use crate::types::{Block, BlockVote, EpochId, FinalityCertificate, Nullifier, SpendTransaction};

/// Messages gossiped on per-zone spend topics (high-volume, loss-tolerant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZephyrZoneMessage {
    SubmitSpend(SpendTransaction),
    SubmitSpendBatch(Vec<SpendTransaction>),
}

/// Messages gossiped on per-zone consensus topics (low-volume, latency-sensitive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZephyrConsensusMessage {
    Proposal(Block),
    Vote(BlockVote),
    Reject(SpendReject),
}

/// Messages gossiped on the global topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZephyrGlobalMessage {
    Certificate {
        cert: FinalityCertificate,
        /// Hex-encoded nullifier prefixes of the transactions in the certified
        /// block. Allows nodes that did not see the zone-level proposal to
        /// display transaction data in the dashboard.
        #[serde(default)]
        tx_nullifiers: Vec<String>,
    },
    EpochAnnounce(EpochAnnouncement),
}

/// Notification that a spend was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendReject {
    pub nullifier: Nullifier,
    pub reason: RejectReason,
}

/// Why a spend was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    InvalidProof,
    DuplicateNullifier,
    InvalidCommitment,
}

/// Announcement of a new epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochAnnouncement {
    pub epoch: EpochId,
    #[serde(with = "serde_bytes")]
    pub randomness_seed: [u8; 32],
    pub start_time_ms: u64,
}
