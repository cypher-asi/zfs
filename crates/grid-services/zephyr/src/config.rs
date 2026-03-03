use grid_programs_zephyr::ValidatorInfo;
use serde::{Deserialize, Serialize};

/// Configuration for the Zephyr service.
///
/// Invariants:
/// - `total_zones` > 0
/// - `committee_size` > 0
/// - `quorum_threshold` <= `committee_size`
/// - `epoch_duration_ms` > 0
/// - `round_interval_ms` > 0
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZephyrConfig {
    /// Number of zones (A). Nullifiers are routed via `H(N) mod A`.
    #[serde(default = "default_total_zones")]
    pub total_zones: u32,
    /// Committee size per zone (k).
    #[serde(default = "default_committee_size")]
    pub committee_size: usize,
    /// Epoch duration in milliseconds (E).
    #[serde(default = "default_epoch_duration_ms")]
    pub epoch_duration_ms: u64,
    /// Proposal round interval in milliseconds (T_round).
    #[serde(default = "default_round_interval_ms")]
    pub round_interval_ms: u64,
    /// Quorum threshold for certificate assembly: `ceil(2k/3)`.
    #[serde(default = "default_quorum_threshold")]
    pub quorum_threshold: usize,
    /// Maximum spends per block.
    #[serde(default = "default_max_block_size")]
    pub max_block_size: usize,
    /// Number of round ticks before a stalled round times out and the leader
    /// rotates.  Prevents permanent zone stalls when quorum is not reached.
    #[serde(default = "default_round_timeout_ticks")]
    pub round_timeout_ticks: u32,
    /// Maximum number of out-of-order certificates buffered per zone while
    /// waiting for their parent to arrive.  At ~7 blocks/sec, 64 entries
    /// cover ~9 seconds of lag.  Stale certs from dead forks are purged
    /// when the buffer exceeds 75% of this cap.
    #[serde(default = "default_max_pending_certs")]
    pub max_pending_certs: usize,
    /// Genesis randomness seed (R_0).
    #[serde(default, with = "hex_bytes")]
    pub initial_randomness: [u8; 32],
    /// Static validator list (MVP).
    #[serde(default)]
    pub validators: Vec<ValidatorInfo>,
    /// When true, this node participates as a solo validator using its own
    /// identity. The validator list is auto-populated on start.
    #[serde(default)]
    pub self_validate: bool,
}

fn default_total_zones() -> u32 {
    4
}
fn default_committee_size() -> usize {
    5
}
fn default_epoch_duration_ms() -> u64 {
    120_000
}
fn default_round_interval_ms() -> u64 {
    100
}
fn default_quorum_threshold() -> usize {
    4
}
fn default_max_block_size() -> usize {
    512
}
fn default_round_timeout_ticks() -> u32 {
    50
}
fn default_max_pending_certs() -> usize {
    64
}

impl Default for ZephyrConfig {
    fn default() -> Self {
        Self {
            total_zones: default_total_zones(),
            committee_size: default_committee_size(),
            epoch_duration_ms: default_epoch_duration_ms(),
            round_interval_ms: default_round_interval_ms(),
            quorum_threshold: default_quorum_threshold(),
            max_block_size: default_max_block_size(),
            round_timeout_ticks: default_round_timeout_ticks(),
            max_pending_certs: default_max_pending_certs(),
            initial_randomness: [0u8; 32],
            validators: vec![],
            self_validate: false,
        }
    }
}

mod hex_bytes {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}
