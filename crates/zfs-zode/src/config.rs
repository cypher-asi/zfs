use std::collections::HashSet;
use std::path::PathBuf;

use zfs_core::{ProgramId, SectorId};
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

/// Per-Zode filter controlling which sectors are served.
///
/// Applied after the program-level topic check. `All` means every sector
/// under a subscribed program is accepted. `AllowList` restricts to an
/// explicit set of sector IDs.
#[derive(Debug, Clone, Default)]
pub enum SectorFilter {
    #[default]
    All,
    AllowList(HashSet<SectorId>),
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
    /// Sector-specific limits.
    pub sector_limits: SectorLimitsConfig,
    /// Per-sector filter (default: accept all).
    pub sector_filter: SectorFilter,
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

/// Sector protocol storage limits.
#[derive(Debug, Clone)]
pub struct SectorLimitsConfig {
    /// Maximum payload size per sector entry (bytes). Default: 256 KB.
    pub max_slot_size_bytes: u64,
    /// Maximum total storage per program (bytes). `None` = unlimited.
    pub max_per_program_bytes: Option<u64>,
}

impl Default for SectorLimitsConfig {
    fn default() -> Self {
        Self {
            max_slot_size_bytes: 256 * 1024,
            max_per_program_bytes: None,
        }
    }
}

impl Default for ZodeConfig {
    fn default() -> Self {
        Self {
            storage: StorageConfig::new(PathBuf::from("zfs-zode-data")),
            default_programs: DefaultProgramsConfig::default(),
            topics: HashSet::new(),
            sector_limits: SectorLimitsConfig::default(),
            sector_filter: SectorFilter::default(),
            network: NetworkConfig::default(),
        }
    }
}
