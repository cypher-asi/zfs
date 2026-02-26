use std::collections::VecDeque;
use std::sync::Arc;

use grid_core::{ProgramId, SectorId};
use grid_crypto::SectorKey;
use grid_programs_interlink::interlink::ChannelId;
use zode::ZodeStatus;

pub(crate) const MAX_LOG_ENTRIES: usize = 500;

pub(crate) enum PeerEvent {
    Connected(String),
    Disconnected(String),
    Discovered(String),
}

#[derive(Default)]
pub(crate) struct AppState {
    pub status: Option<ZodeStatus>,
    pub log_entries: VecDeque<String>,
    pub listen_addr: Option<String>,
    pub peer_events: VecDeque<PeerEvent>,
}

impl AppState {
    pub fn snapshot(&mut self) -> StateSnapshot {
        StateSnapshot {
            status: self.status.clone(),
            log_entries: self.log_entries.iter().cloned().collect(),
            listen_addr: self.listen_addr.clone(),
            peer_events: self.peer_events.drain(..).collect(),
        }
    }
}

pub(crate) struct StateSnapshot {
    pub status: Option<ZodeStatus>,
    pub log_entries: Vec<String>,
    pub listen_addr: Option<String>,
    pub peer_events: Vec<PeerEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Status,
    Storage,
    Peers,
    Log,
    Interlink,
    Info,
    Settings,
    Identity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppPhase {
    ProfileSelect,
    Unlock { profile_id: String },
    Unlocking { profile_id: String },
    Revealing,
    Running,
}

pub(crate) struct IdentityState {
    pub shares: Vec<zid::ShamirShare>,
    pub threshold: usize,
    pub identity_id: [u8; 16],
    pub verifying_key: Option<zid::IdentityVerifyingKey>,
    pub did: Option<String>,
    pub show_shares: bool,
    pub recovery_mode: bool,
    pub recovery_inputs: Vec<String>,
    pub recovery_input: String,
    pub machine_keys: Vec<DerivedMachineKey>,
    pub error: Option<String>,
    pub pending_save: bool,
    pub save_password: String,
    pub save_profile_name: String,
    pub save_status: Option<String>,
}

impl Default for IdentityState {
    fn default() -> Self {
        Self {
            shares: Vec::new(),
            threshold: 3,
            identity_id: [0u8; 16],
            verifying_key: None,
            did: None,
            show_shares: false,
            recovery_mode: false,
            recovery_inputs: Vec::new(),
            recovery_input: String::new(),
            machine_keys: Vec::new(),
            error: None,
            pending_save: false,
            save_password: String::new(),
            save_profile_name: String::from("Default"),
            save_status: None,
        }
    }
}

#[allow(dead_code)]
pub(crate) struct DerivedMachineKey {
    pub machine_id: [u8; 16],
    pub epoch: u64,
    pub capabilities: zid::MachineKeyCapabilities,
    pub did: String,
    pub public_key: zid::MachinePublicKey,
    pub keypair: Arc<zid::MachineKeyPair>,
}

pub(crate) struct DisplayMessage {
    pub sender: String,
    pub content: String,
    pub timestamp_ms: u64,
    pub signature_status: SignatureStatus,
}

/// Signature verification status for display purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignatureStatus {
    None,
    Verified,
    Failed,
}

/// Incremental update carrying only newly-discovered messages.
pub(crate) struct InterlinkUpdate {
    pub new_messages: Vec<DisplayMessage>,
    pub error: Option<String>,
}

pub(crate) struct InterlinkState {
    pub messages: Vec<DisplayMessage>,
    pub compose: String,
    pub sector_key: Option<SectorKey>,
    pub machine_did: String,
    pub signing_keypair: Option<Arc<zid::MachineKeyPair>>,
    pub channel_id: Option<ChannelId>,
    pub program_id: Option<ProgramId>,
    /// Per-channel sector ID (one sector per channel in append model).
    pub sector_id: Option<SectorId>,
    pub prover: Option<Box<grid_proof_groth16::Groth16ShapeProver>>,
    pub error: Option<String>,
    pub initialized: bool,
    pub scroll_to_bottom: bool,
    pub focus_compose: bool,
    pub update_rx: Option<tokio::sync::mpsc::Receiver<InterlinkUpdate>>,
    pub refresh_tx: Option<tokio::sync::mpsc::Sender<()>>,
}

impl InterlinkState {
    pub fn error_only(msg: &str) -> Self {
        Self {
            messages: Vec::new(),
            compose: String::new(),
            sector_key: None,
            machine_did: String::new(),
            signing_keypair: None,
            channel_id: None,
            program_id: None,
            sector_id: None,
            prover: None,
            error: Some(msg.to_string()),
            initialized: true,
            scroll_to_bottom: false,
            focus_compose: false,
            update_rx: None,
            refresh_tx: None,
        }
    }
}
