use serde::{Deserialize, Serialize};

pub type ZoneId = u32;
pub type EpochId = u64;

/// A note commitment: `C = Poseidon(value || owner_pubkey || r)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteCommitment(#[serde(with = "serde_bytes")] pub [u8; 32]);

/// A nullifier: `N = Poseidon(owner_secret || C)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nullifier(#[serde(with = "serde_bytes")] pub [u8; 32]);

impl AsRef<[u8]> for Nullifier {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for NoteCommitment {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// An output note in a spend transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteOutput {
    pub commitment: NoteCommitment,
    #[serde(with = "serde_bytes")]
    pub encrypted_data: Vec<u8>,
}

/// A spend transaction with ZK proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendTransaction {
    pub input_commitment: NoteCommitment,
    pub nullifier: Nullifier,
    pub outputs: Vec<NoteOutput>,
    #[serde(with = "serde_bytes")]
    pub proof: Vec<u8>,
    pub public_signals: Vec<[u8; 32]>,
}

/// A validator in the global pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorInfo {
    #[serde(with = "serde_bytes")]
    pub validator_id: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub pubkey: [u8; 32],
    pub p2p_endpoint: String,
}

/// Header of a finalized block in a zone chain.
///
/// `block_hash = SHA-256(canonical(BlockHeader))` — since `parent_hash` is in
/// the header, the hash inherently chains blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub zone_id: ZoneId,
    pub epoch: EpochId,
    pub height: u64,
    #[serde(with = "serde_bytes")]
    pub parent_hash: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub transactions_root: [u8; 32],
    pub timestamp_ms: u64,
    #[serde(with = "serde_bytes")]
    pub proposer_id: [u8; 32],
}

/// A block proposed by a committee leader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<SpendTransaction>,
    #[serde(with = "serde_bytes")]
    pub block_hash: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub proposer_sig: Vec<u8>,
}

/// A vote on a block proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockVote {
    pub zone_id: ZoneId,
    pub epoch: EpochId,
    #[serde(with = "serde_bytes")]
    pub block_hash: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub voter_id: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
}

/// A finality certificate for a block. The certified `block_hash` is the new zone head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalityCertificate {
    pub zone_id: ZoneId,
    pub epoch: EpochId,
    #[serde(with = "serde_bytes")]
    pub parent_hash: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub block_hash: [u8; 32],
    pub signatures: Vec<CertSignature>,
}

/// A validator's signature in a finality certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertSignature {
    #[serde(with = "serde_bytes")]
    pub validator_id: [u8; 32],
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
}
