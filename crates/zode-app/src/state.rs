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

pub(crate) struct IdentityState {
    pub shares: Vec<zero_neural::ShamirShare>,
    pub threshold: usize,
    pub identity_id: [u8; 16],
    pub verifying_key: Option<zero_neural::IdentityVerifyingKey>,
    pub did: Option<String>,
    pub show_shares: bool,
    pub recovery_mode: bool,
    pub recovery_inputs: Vec<String>,
    pub recovery_input: String,
    pub machine_keys: Vec<DerivedMachineKey>,
    pub new_machine_id_hex: String,
    pub new_epoch: u64,
    pub cap_sign: bool,
    pub cap_encrypt: bool,
    pub cap_store: bool,
    pub cap_fetch: bool,
    pub error: Option<String>,
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
            new_machine_id_hex: String::new(),
            new_epoch: 1,
            cap_sign: true,
            cap_encrypt: true,
            cap_store: false,
            cap_fetch: false,
            error: None,
        }
    }
}

#[allow(dead_code)]
pub(crate) struct DerivedMachineKey {
    pub machine_id: [u8; 16],
    pub epoch: u64,
    pub capabilities: zero_neural::MachineKeyCapabilities,
    pub did: String,
    pub public_key: zero_neural::MachinePublicKey,
    pub keypair: Arc<zero_neural::MachineKeyPair>,
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
    pub sector_key: SectorKey,
    pub machine_did: String,
    pub signing_keypair: Arc<zero_neural::MachineKeyPair>,
    pub channel_id: ChannelId,
    pub program_id: ProgramId,
    /// Per-channel sector ID (one sector per channel in append model).
    pub sector_id: SectorId,
    pub prover: Box<grid_proof_groth16::Groth16ShapeProver>,
    pub error: Option<String>,
    pub initialized: bool,
    pub scroll_to_bottom: bool,
    pub focus_compose: bool,
    pub update_rx: tokio::sync::mpsc::Receiver<InterlinkUpdate>,
    pub refresh_tx: tokio::sync::mpsc::Sender<()>,
}
