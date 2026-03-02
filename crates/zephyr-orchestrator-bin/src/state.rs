use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// Top-level application phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppPhase {
    Launch,
    Running,
    ShuttingDown,
}

/// Dashboard tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Dashboard,
    Nodes,
    Topology,
    Log,
}

impl Tab {
    pub const ALL: &[Tab] = &[Tab::Dashboard, Tab::Nodes, Tab::Topology, Tab::Log];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Nodes => "Nodes",
            Self::Topology => "Topology",
            Self::Log => "Log",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Dashboard => egui_phosphor::regular::CHART_BAR,
            Self::Nodes => egui_phosphor::regular::COMPUTER_TOWER,
            Self::Topology => egui_phosphor::regular::GRAPH,
            Self::Log => egui_phosphor::regular::TERMINAL,
        }
    }
}

/// Network preset for launching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NetworkPreset {
    Minimal,
    Standard,
    Large,
    Custom {
        validators: usize,
        zones: u32,
        committee_size: usize,
    },
}

impl NetworkPreset {
    pub fn label(&self) -> &str {
        match self {
            Self::Minimal => "Minimal",
            Self::Standard => "Standard",
            Self::Large => "Large",
            Self::Custom { .. } => "Custom",
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Minimal => "3 validators, 2 zones",
            Self::Standard => "5 validators, 4 zones",
            Self::Large => "10 validators, 8 zones",
            Self::Custom { .. } => "Custom configuration",
        }
    }

    pub fn validators(&self) -> usize {
        match self {
            Self::Minimal => 3,
            Self::Standard => 5,
            Self::Large => 10,
            Self::Custom { validators, .. } => *validators,
        }
    }

    pub fn zones(&self) -> u32 {
        match self {
            Self::Minimal => 2,
            Self::Standard => 4,
            Self::Large => 8,
            Self::Custom { zones, .. } => *zones,
        }
    }

    pub fn committee_size(&self) -> usize {
        match self {
            Self::Minimal => 3,
            Self::Standard => 3,
            Self::Large => 5,
            Self::Custom { committee_size, .. } => *committee_size,
        }
    }
}

/// Per-node live state, updated by polling.
pub(crate) struct NodeState {
    pub node_id: usize,
    pub zode_id: String,
    pub status: Option<zode::ZodeStatus>,
    pub assigned_zones: Vec<u32>,
    pub is_leader_in: Vec<u32>,
    pub mempool_sizes: HashMap<u32, usize>,
    pub last_update: Instant,
}

impl NodeState {
    pub fn new(node_id: usize) -> Self {
        Self {
            node_id,
            zode_id: String::new(),
            status: None,
            assigned_zones: Vec::new(),
            is_leader_in: Vec::new(),
            mempool_sizes: HashMap::new(),
            last_update: Instant::now(),
        }
    }
}

/// Aggregated network view.
pub(crate) struct NetworkSnapshot {
    pub total_zones: u32,
    pub current_epoch: u64,
    pub epoch_progress_pct: f32,
    pub zone_heads: HashMap<u32, [u8; 32]>,
    pub certificates_produced: u64,
    pub spends_processed: u64,
    pub total_peers: usize,
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        Self {
            total_zones: 0,
            current_epoch: 0,
            epoch_progress_pct: 0.0,
            zone_heads: HashMap::new(),
            certificates_produced: 0,
            spends_processed: 0,
            total_peers: 0,
        }
    }
}

/// Aggregated log entry from any node.
pub(crate) struct AggregatedLogEntry {
    pub node_id: usize,
    pub line: String,
    pub level: LogLevel,
    pub timestamp: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Debug => "DEBUG",
        }
    }
}

/// A recently submitted transaction for the activity feed.
pub(crate) struct RecentTransaction {
    pub nullifier_hex: String,
    pub zone_id: u32,
    pub timestamp: Instant,
}

/// Traffic generator statistics.
#[derive(Default)]
pub(crate) struct TrafficStats {
    pub total_submitted: u64,
    pub recent: VecDeque<RecentTransaction>,
}

/// Shared mutable state polled by the UI.
pub(crate) struct AppState {
    pub phase: AppPhase,
    pub nodes: Vec<NodeState>,
    pub network: NetworkSnapshot,
    pub log_entries: Vec<AggregatedLogEntry>,
    pub launch_start: Option<Instant>,
    pub auto_traffic: bool,
    pub traffic_rate: f32,
    pub traffic_stats: TrafficStats,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            phase: AppPhase::Launch,
            nodes: Vec::new(),
            network: NetworkSnapshot::default(),
            log_entries: Vec::new(),
            launch_start: None,
            auto_traffic: false,
            traffic_rate: 1.0,
            traffic_stats: TrafficStats::default(),
        }
    }
}
