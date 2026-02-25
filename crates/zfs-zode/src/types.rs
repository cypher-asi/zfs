use std::fmt;

use crate::metrics::MetricsSnapshot;

/// Structured log events emitted by the Zode for UI consumption.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// Zode started and is serving.
    Started { listen_addr: String },
    /// A new peer connected.
    PeerConnected(String),
    /// A peer disconnected.
    PeerDisconnected(String),
    /// A new peer was discovered via DHT.
    PeerDiscovered(String),
    /// A sector append request was processed.
    SectorAppendProcessed {
        program_id: String,
        sector_id: String,
        index: Option<u64>,
        accepted: bool,
    },
    /// A sector read-log request was processed.
    SectorReadLogProcessed {
        program_id: String,
        sector_id: String,
        entries: usize,
    },
    /// A gossip sector append was received and stored (or rejected).
    GossipSectorReceived {
        program_id: String,
        sector_id: String,
        accepted: bool,
    },
    /// The Zode is shutting down.
    ShuttingDown,
}

impl fmt::Display for LogEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started { listen_addr } => write!(f, "[STARTED] listening on {listen_addr}"),
            Self::PeerConnected(peer) => write!(f, "[PEER+] {peer}"),
            Self::PeerDisconnected(peer) => write!(f, "[PEER-] {peer}"),
            Self::PeerDiscovered(peer) => write!(f, "[DHT] discovered {peer}"),
            Self::SectorAppendProcessed {
                program_id,
                sector_id,
                index,
                accepted,
            } => {
                let status = if *accepted { "OK" } else { "REJECT" };
                let idx = index.map(|i| format!(" idx={i}")).unwrap_or_default();
                write!(
                    f,
                    "[SECTOR APPEND {status}] prog={} sid={}{}",
                    &program_id[..8.min(program_id.len())],
                    &sector_id[..8.min(sector_id.len())],
                    idx,
                )
            }
            Self::SectorReadLogProcessed {
                program_id,
                sector_id,
                entries,
            } => {
                write!(
                    f,
                    "[SECTOR READ] prog={} sid={} entries={entries}",
                    &program_id[..8.min(program_id.len())],
                    &sector_id[..8.min(sector_id.len())],
                )
            }
            Self::GossipSectorReceived {
                program_id,
                sector_id,
                accepted,
            } => {
                let status = if *accepted { "STORED" } else { "REJECT" };
                write!(
                    f,
                    "[GOSSIP {status}] prog={} sid={}",
                    &program_id[..8.min(program_id.len())],
                    &sector_id[..8.min(sector_id.len())]
                )
            }
            Self::ShuttingDown => write!(f, "[SHUTDOWN] Zode shutting down"),
        }
    }
}

/// Severity / category for a formatted log line, used by UI crates to pick
/// colours without duplicating prefix-matching logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Reject,
    Gossip,
    Discovery,
    PeerConnect,
    PeerDisconnect,
    Shutdown,
    Normal,
}

impl LogLevel {
    pub fn from_log_line(line: &str) -> Self {
        if line.starts_with("[SECTOR APPEND REJECT") || line.starts_with("[REJECT") {
            Self::Reject
        } else if line.starts_with("[GOSSIP") {
            Self::Gossip
        } else if line.starts_with("[DHT") {
            Self::Discovery
        } else if line.starts_with("[PEER+") {
            Self::PeerConnect
        } else if line.starts_with("[PEER-") {
            Self::PeerDisconnect
        } else if line.starts_with("[SHUTDOWN") {
            Self::Shutdown
        } else {
            Self::Normal
        }
    }
}

/// Status snapshot of the running Zode.
#[derive(Debug, Clone)]
pub struct ZodeStatus {
    /// The local Zode ID.
    pub zode_id: String,
    /// Number of connected Zodes.
    pub peer_count: u64,
    /// Connected Zode IDs.
    pub connected_peers: Vec<String>,
    /// Subscribed program topics.
    pub topics: Vec<String>,
    /// Metrics snapshot.
    pub metrics: MetricsSnapshot,
}
