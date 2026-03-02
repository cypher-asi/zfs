use serde::{Deserialize, Serialize};

use crate::types::{Block, BlockVote, EpochId, FinalityCertificate, Nullifier, SpendTransaction};

/// Messages gossiped on zone topics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZephyrZoneMessage {
    SubmitSpend(SpendTransaction),
    Proposal(Block),
    Vote(BlockVote),
    Reject(SpendReject),
}

/// Messages gossiped on the global topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZephyrGlobalMessage {
    Certificate(FinalityCertificate),
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
