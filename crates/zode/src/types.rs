use std::fmt;

use crate::metrics::MetricsSnapshot;

/// Produce a short display form of a Zode ID showing the last 6
/// characters of the unique portion (after the common `Zx12D3KooW`
/// multicodec prefix shared by all libp2p PeerIds).
fn short_zode_id(id: &str) -> String {
    const PREFIX: &str = "Zx12D3KooW";
    if id.starts_with(PREFIX) {
        let unique = &id[PREFIX.len()..];
        let n = 6.min(unique.len());
        format!("Zx..{}", &unique[unique.len() - n..])
    } else if id.len() > 10 {
        format!("{}..{}", &id[..4], &id[id.len() - 6..])
    } else {
        id.to_string()
    }
}

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
        sender: Option<String>,
        program_id: String,
        sector_id: String,
        result: GossipAppendResult,
    },
    /// The RPC server has started listening.
    RpcStarted { bind_addr: String },
    /// An RPC request was processed.
    RpcRequest { method: String, success: bool },
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
                sender,
                program_id,
                sector_id,
                result,
            } => {
                let prog = &program_id[..8.min(program_id.len())];
                let sid = &sector_id[..8.min(sector_id.len())];
                let from = sender
                    .as_deref()
                    .map(short_zode_id)
                    .unwrap_or_else(|| "unknown".into());
                match result {
                    GossipAppendResult::Stored => {
                        write!(f, "[GOSSIP STORED] from={from} prog={prog} sid={sid}")
                    }
                    GossipAppendResult::Duplicate => {
                        write!(f, "[GOSSIP DUP] from={from} prog={prog} sid={sid}")
                    }
                    GossipAppendResult::Rejected(reason) => {
                        write!(
                            f,
                            "[GOSSIP REJECT] from={from} prog={prog} sid={sid}: {reason}"
                        )
                    }
                }
            }
            Self::RpcStarted { bind_addr } => {
                write!(f, "[RPC STARTED] listening on {bind_addr}")
            }
            Self::RpcRequest { method, success } => {
                let status = if *success { "OK" } else { "ERR" };
                write!(f, "[RPC] {method} {status}")
            }
            Self::ShuttingDown => write!(f, "[SHUTDOWN] ZODE shutting down"),
        }
    }
}

/// Outcome of a gossip sector append.
#[derive(Debug, Clone)]
pub enum GossipAppendResult {
    /// The entry was stored successfully.
    Stored,
    /// The entry already existed at this index (idempotent no-op).
    Duplicate,
    /// The entry was rejected.
    Rejected(GossipRejectReason),
}

impl GossipAppendResult {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Stored | Self::Duplicate)
    }
}

/// Why a gossip sector append was rejected.
#[derive(Debug, Clone)]
pub enum GossipRejectReason {
    /// The program ID is not in the subscribed topic set.
    ProgramNotSubscribed,
    /// The sector ID is filtered out by the sector policy.
    SectorFiltered,
    /// The payload exceeds the maximum entry size.
    EntryTooLarge { size: usize, max: u64 },
    /// A shape proof is required but was not included.
    ProofMissing,
    /// The ciphertext in the payload could not be parsed for hashing.
    CiphertextMalformed,
    /// The ciphertext hash in the proof does not match the payload.
    CiphertextHashMismatch,
    /// The Groth16/ZK proof did not verify.
    ProofVerificationFailed { detail: String },
    /// Writing to storage failed.
    StorageError { detail: String },
}

impl fmt::Display for GossipRejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramNotSubscribed => write!(f, "program not subscribed"),
            Self::SectorFiltered => write!(f, "sector filtered by policy"),
            Self::EntryTooLarge { size, max } => {
                write!(f, "entry too large ({size} bytes, max {max})")
            }
            Self::ProofMissing => write!(f, "shape proof required but missing"),
            Self::CiphertextMalformed => write!(f, "ciphertext malformed for hash"),
            Self::CiphertextHashMismatch => write!(f, "ciphertext hash mismatch"),
            Self::ProofVerificationFailed { detail } => {
                write!(f, "proof verification failed: {detail}")
            }
            Self::StorageError { detail } => write!(f, "storage error: {detail}"),
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
    Rpc,
    Shutdown,
    Normal,
}

impl LogLevel {
    pub fn from_log_line(line: &str) -> Self {
        if line.starts_with("[SECTOR APPEND REJECT")
            || line.starts_with("[REJECT")
            || line.starts_with("[GOSSIP REJECT")
        {
            Self::Reject
        } else if line.starts_with("[GOSSIP") {
            Self::Gossip
        } else if line.starts_with("[DHT") {
            Self::Discovery
        } else if line.starts_with("[PEER+") {
            Self::PeerConnect
        } else if line.starts_with("[PEER-") {
            Self::PeerDisconnect
        } else if line.starts_with("[RPC") {
            Self::Rpc
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
    /// Whether the RPC server is enabled and running.
    pub rpc_enabled: bool,
    /// RPC server bind address (e.g. "127.0.0.1:4690"), if running.
    pub rpc_addr: Option<String>,
    /// Whether the RPC server requires API key authentication.
    pub rpc_auth_required: bool,
}
