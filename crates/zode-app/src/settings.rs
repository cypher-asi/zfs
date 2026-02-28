use std::collections::HashSet;
use std::path::{Path, PathBuf};

use grid_core::ProgramId;
use grid_net::NetworkConfig;
use grid_storage::StorageConfig;
use serde::{Deserialize, Serialize};
use zode::{DefaultProgramsConfig, RpcConfig, ZodeConfig};

/// Persisted portion of settings (everything except transient UI input buffers).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedSettings {
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    #[serde(default)]
    pub bootstrap_peers: Vec<String>,
    #[serde(default = "default_true")]
    pub enable_relay: bool,
    #[serde(default = "default_relay_peers")]
    pub relay_peers: Vec<String>,
    #[serde(default = "default_true")]
    pub enable_zid: bool,
    #[serde(default = "default_true")]
    pub enable_interlink: bool,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default = "default_true")]
    pub enable_kademlia: bool,
    #[serde(default = "default_true")]
    pub kademlia_server_mode: bool,
    #[serde(default = "default_random_walk")]
    pub random_walk_interval_secs: u64,
    #[serde(default)]
    pub enable_rpc: bool,
    #[serde(default = "default_rpc_bind")]
    pub rpc_bind_addr: String,
    #[serde(default)]
    pub rpc_api_key: String,
    /// Peers seen in previous sessions, merged into bootstrap on next start.
    #[serde(default)]
    pub known_peers: Vec<String>,
}

fn default_data_dir() -> String {
    ".zode/data".into()
}
fn default_listen_addr() -> String {
    "/ip4/127.0.0.1/udp/3690/quic-v1".into()
}
fn default_true() -> bool {
    true
}
fn default_random_walk() -> u64 {
    30
}
fn default_rpc_bind() -> String {
    "127.0.0.1:4690".into()
}
fn default_relay_peers() -> Vec<String> {
    vec!["/ip4/3.129.15.45/tcp/3691".into()]
}

pub(crate) struct Settings {
    pub data_dir: String,
    pub listen_addr: String,
    pub bootstrap_input: String,
    pub bootstrap_peers: Vec<String>,
    pub enable_relay: bool,
    pub relay_input: String,
    pub relay_peers: Vec<String>,
    pub enable_zid: bool,
    pub enable_interlink: bool,
    pub topic_input: String,
    pub topics: Vec<String>,
    pub enable_kademlia: bool,
    pub kademlia_server_mode: bool,
    pub random_walk_interval_secs: u64,
    pub enable_rpc: bool,
    pub rpc_bind_addr: String,
    pub rpc_api_key: String,
    /// Peers from previous sessions.
    pub known_peers: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: ".zode/data".into(),
            listen_addr: "/ip4/127.0.0.1/udp/3690/quic-v1".into(),
            bootstrap_input: String::new(),
            bootstrap_peers: Vec::new(),
            enable_relay: true,
            relay_input: String::new(),
            relay_peers: vec!["/ip4/3.129.15.45/tcp/3691".into()],
            enable_zid: true,
            enable_interlink: true,
            topic_input: String::new(),
            topics: Vec::new(),
            enable_kademlia: true,
            kademlia_server_mode: true,
            random_walk_interval_secs: 30,
            enable_rpc: false,
            rpc_bind_addr: "127.0.0.1:4690".into(),
            rpc_api_key: String::new(),
            known_peers: Vec::new(),
        }
    }
}

impl Settings {
    pub fn to_persisted(&self) -> PersistedSettings {
        PersistedSettings {
            data_dir: self.data_dir.clone(),
            listen_addr: self.listen_addr.clone(),
            bootstrap_peers: self.bootstrap_peers.clone(),
            enable_relay: self.enable_relay,
            relay_peers: self.relay_peers.clone(),
            enable_zid: self.enable_zid,
            enable_interlink: self.enable_interlink,
            topics: self.topics.clone(),
            enable_kademlia: self.enable_kademlia,
            kademlia_server_mode: self.kademlia_server_mode,
            random_walk_interval_secs: self.random_walk_interval_secs,
            enable_rpc: self.enable_rpc,
            rpc_bind_addr: self.rpc_bind_addr.clone(),
            rpc_api_key: self.rpc_api_key.clone(),
            known_peers: self.known_peers.clone(),
        }
    }

    pub fn apply_persisted(&mut self, p: PersistedSettings) {
        self.data_dir = p.data_dir;
        self.listen_addr = p.listen_addr;
        self.bootstrap_peers = p.bootstrap_peers;
        self.enable_relay = p.enable_relay;
        self.relay_peers = p.relay_peers;
        self.enable_zid = p.enable_zid;
        self.enable_interlink = p.enable_interlink;
        self.topics = p.topics;
        self.enable_kademlia = p.enable_kademlia;
        self.kademlia_server_mode = p.kademlia_server_mode;
        self.random_walk_interval_secs = p.random_walk_interval_secs;
        self.enable_rpc = p.enable_rpc;
        self.rpc_bind_addr = p.rpc_bind_addr;
        self.rpc_api_key = p.rpc_api_key;
        self.known_peers = p.known_peers;
    }

    /// Load persisted settings from a JSON file. Returns default if the file
    /// does not exist or cannot be parsed.
    pub fn load_from(path: &Path) -> Self {
        let mut s = Self::default();
        if let Ok(data) = std::fs::read_to_string(path) {
            if let Ok(p) = serde_json::from_str::<PersistedSettings>(&data) {
                s.apply_persisted(p);
            }
        }
        s
    }

    /// Persist current settings to a JSON file.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create settings dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(&self.to_persisted())
            .map_err(|e| format!("serialize settings: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("write settings: {e}"))
    }

    /// Merge currently connected peers into `known_peers` for next startup.
    pub fn remember_peers(&mut self, connected: &[String]) {
        for peer in connected {
            if !self.known_peers.contains(peer) {
                self.known_peers.push(peer.clone());
            }
        }
        const MAX_KNOWN: usize = 200;
        if self.known_peers.len() > MAX_KNOWN {
            let excess = self.known_peers.len() - MAX_KNOWN;
            self.known_peers.drain(..excess);
        }
    }
}

impl Settings {
    pub fn build_config(&self) -> Result<ZodeConfig, String> {
        let listen_addr: grid_net::Multiaddr = self
            .listen_addr
            .parse()
            .map_err(|e| format!("Bad listen address: {e}"))?;

        let mut bootstrap = self.parse_bootstrap_peers()?;
        let known = self.parse_known_peers();
        for addr in known {
            if !bootstrap.contains(&addr) {
                bootstrap.push(addr);
            }
        }
        let relay = self.parse_relay_peers()?;
        let topic_set = self.parse_topics()?;

        let kad_mode = if self.kademlia_server_mode {
            grid_net::KademliaMode::Server
        } else {
            grid_net::KademliaMode::Client
        };

        let discovery = grid_net::DiscoveryConfig {
            enable_kademlia: self.enable_kademlia,
            kademlia_mode: kad_mode,
            random_walk_interval: std::time::Duration::from_secs(self.random_walk_interval_secs),
            ..Default::default()
        };

        let network = NetworkConfig::new(listen_addr)
            .with_bootstrap_peers(bootstrap)
            .with_relay(grid_net::RelayConfig {
                enabled: self.enable_relay || !relay.is_empty(),
                relay_peers: relay,
            })
            .with_discovery(discovery);
        let storage = StorageConfig::new(PathBuf::from(&self.data_dir));

        let rpc = if self.enable_rpc {
            let bind_addr = self
                .rpc_bind_addr
                .parse()
                .map_err(|e| format!("Bad RPC bind address: {e}"))?;
            RpcConfig {
                enabled: true,
                bind_addr,
                api_key: if self.rpc_api_key.is_empty() {
                    None
                } else {
                    Some(self.rpc_api_key.clone())
                },
            }
        } else {
            RpcConfig::default()
        };

        Ok(ZodeConfig {
            storage,
            default_programs: DefaultProgramsConfig {
                zid: self.enable_zid,
                interlink: self.enable_interlink,
            },
            topics: topic_set,
            sector_limits: Default::default(),
            sector_filter: Default::default(),
            network,
            rpc,
        })
    }

    fn parse_bootstrap_peers(&self) -> Result<Vec<grid_net::Multiaddr>, String> {
        self.bootstrap_peers
            .iter()
            .map(|s| {
                grid_net::strip_zx_multiaddr(s)
                    .parse()
                    .map_err(|e| format!("Bad bootstrap addr '{s}': {e}"))
            })
            .collect()
    }

    fn parse_topics(&self) -> Result<HashSet<ProgramId>, String> {
        self.topics
            .iter()
            .map(|hex| ProgramId::from_hex(hex).map_err(|e| format!("Bad topic '{hex}': {e}")))
            .collect()
    }

    fn parse_relay_peers(&self) -> Result<Vec<grid_net::Multiaddr>, String> {
        self.relay_peers
            .iter()
            .map(|s| {
                grid_net::strip_zx_multiaddr(s)
                    .parse()
                    .map_err(|e| format!("Bad relay addr '{s}': {e}"))
            })
            .collect()
    }

    /// Best-effort parse of known peers; silently drops unparseable entries.
    fn parse_known_peers(&self) -> Vec<grid_net::Multiaddr> {
        self.known_peers
            .iter()
            .filter_map(|s| grid_net::strip_zx_multiaddr(s).parse().ok())
            .collect()
    }
}
