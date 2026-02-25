use std::collections::HashSet;
use std::path::PathBuf;

use zfs_core::ProgramId;
use zfs_net::NetworkConfig;
use zfs_programs::{ZChatDescriptor, ZidDescriptor};
use zfs_storage::StorageConfig;

/// Toggle default programs (ZID, Interlink) on or off.
///
/// Both are enabled by default. Disabling a program removes it from the
/// effective topic list so the Zode will neither subscribe to nor serve
/// requests for that program.
#[derive(Debug, Clone)]
pub struct DefaultProgramsConfig {
    /// Enable the ZID (Zero Identity) program. Default: `true`.
    pub zid: bool,
    /// Enable the Interlink program. Default: `true`.
    pub zchat: bool,
}

impl Default for DefaultProgramsConfig {
    fn default() -> Self {
        Self {
            zid: true,
            zchat: true,
        }
    }
}

impl DefaultProgramsConfig {
    /// Collect the ProgramIds of all enabled default programs.
    pub fn enabled_program_ids(&self) -> HashSet<ProgramId> {
        let mut set = HashSet::new();
        if self.zid {
            if let Ok(pid) = ZidDescriptor::v1().program_id() {
                set.insert(pid);
            }
        }
        if self.zchat {
            if let Ok(pid) = ZChatDescriptor::v1().program_id() {
                set.insert(pid);
            }
        }
        set
    }
}

/// Full Zode configuration.
#[derive(Debug, Clone)]
pub struct ZodeConfig {
    /// RocksDB storage configuration.
    pub storage: StorageConfig,
    /// Toggle default programs on or off.
    pub default_programs: DefaultProgramsConfig,
    /// Additional (non-default) program topics to subscribe to.
    pub topics: HashSet<ProgramId>,
    /// Size and policy limits.
    pub limits: LimitsConfig,
    /// Proof verification policy.
    pub proof_policy: ProofPolicyConfig,
    /// Network (libp2p) configuration.
    pub network: NetworkConfig,
}

impl ZodeConfig {
    /// Compute the effective set of subscribed programs:
    /// enabled default programs **union** explicit `topics`.
    pub fn effective_topics(&self) -> HashSet<ProgramId> {
        let mut set = self.default_programs.enabled_program_ids();
        set.extend(&self.topics);
        set
    }
}

/// Size and policy limits for the Zode.
#[derive(Debug, Clone, Default)]
pub struct LimitsConfig {
    /// Maximum ciphertext size per block (bytes). `None` = unlimited.
    pub max_block_size_bytes: Option<u64>,
    /// Maximum total storage per program (bytes). `None` = unlimited.
    pub max_per_program_bytes: Option<u64>,
    /// Maximum total DB size (bytes). Overrides `StorageConfig::max_db_size_bytes`.
    pub max_total_db_bytes: Option<u64>,
}

/// Proof verification policy.
#[derive(Debug, Clone, Default)]
pub struct ProofPolicyConfig {
    /// Whether to require proofs for all programs.
    pub require_proofs: bool,
    /// Programs that require proofs (when `require_proofs` is false).
    pub programs_requiring_proof: HashSet<ProgramId>,
    /// Path for verifier key storage.
    pub verifier_key_path: Option<PathBuf>,
}

impl Default for ZodeConfig {
    fn default() -> Self {
        Self {
            storage: StorageConfig::new(PathBuf::from("zfs-zode-data")),
            default_programs: DefaultProgramsConfig::default(),
            topics: HashSet::new(),
            limits: LimitsConfig::default(),
            proof_policy: ProofPolicyConfig::default(),
            network: NetworkConfig::default(),
        }
    }
}
