use std::collections::{HashMap, HashSet};
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
    #[serde(default = "default_bootstrap_peers")]
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
    /// Per-service configuration keyed by service name (e.g. `"ZEPHYR"`).
    #[serde(default)]
    pub service_configs: HashMap<String, serde_json::Value>,
}

fn default_data_dir() -> String {
    ".zode/data".into()
}
fn default_listen_addr() -> String {
    "/ip4/0.0.0.0/udp/3690/quic-v1".into()
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
fn default_bootstrap_peers() -> Vec<String> {
    vec![grid_net::DEFAULT_RELAY_PEER.into()]
}
fn default_relay_peers() -> Vec<String> {
    vec![grid_net::DEFAULT_RELAY_PEER.into()]
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
    /// Per-service configuration keyed by service name (e.g. `"ZEPHYR"`).
    pub service_configs: HashMap<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: ".zode/data".into(),
            listen_addr: "/ip4/0.0.0.0/udp/3690/quic-v1".into(),
            bootstrap_input: String::new(),
            bootstrap_peers: vec![grid_net::DEFAULT_RELAY_PEER.into()],
            enable_relay: true,
            relay_input: String::new(),
            relay_peers: vec![grid_net::DEFAULT_RELAY_PEER.into()],
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
            service_configs: HashMap::new(),
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
            service_configs: self.service_configs.clone(),
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
        self.service_configs = p.service_configs;
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
            std::fs::create_dir_all(parent).map_err(|e| format!("create settings dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(&self.to_persisted())
            .map_err(|e| format!("serialize settings: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("write settings: {e}"))
    }

    /// Merge currently connected peers into `known_peers` for next startup.
    /// Deduplicates by (peer_id, IP) so that NAT port rotations replace old
    /// entries instead of accumulating stale addresses across sessions.
    pub fn remember_peers(&mut self, connected: &[String]) {
        for peer in connected {
            let sanitized = sanitize_peer_string(peer);
            if let Some(key) = peer_ip_key(&sanitized) {
                self.known_peers
                    .retain(|existing| peer_ip_key(existing).as_ref() != Some(&key));
            }
            self.known_peers.push(sanitized);
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
        let mut relay = self.parse_relay_peers()?;

        // INVARIANT: DEFAULT_RELAY_PEER is a well-formed static multiaddr.
        let default_relay: grid_net::Multiaddr = grid_net::DEFAULT_RELAY_PEER
            .parse()
            .expect("DEFAULT_RELAY_PEER is a valid multiaddr");
        let default_relay_transport = grid_net::addr::strip_all_p2p(&default_relay);

        // Replace any relay/bootstrap entry at the default relay's transport
        // address that has a stale peer ID (e.g. from before a relay key
        // rotation).
        let is_stale_relay = |addr: &grid_net::Multiaddr| -> bool {
            grid_net::addr::strip_all_p2p(addr) == default_relay_transport && *addr != default_relay
        };
        relay.retain(|addr| !is_stale_relay(addr));
        if !relay.contains(&default_relay) {
            relay.push(default_relay.clone());
        }
        bootstrap.retain(|addr| !is_stale_relay(addr));
        if !bootstrap.contains(&default_relay) {
            bootstrap.push(default_relay);
        }
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
            services: Default::default(),
            service_configs: self.service_configs.clone(),
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

    /// Best-effort parse of known peers; sanitizes duplicate `/p2p/` segments
    /// and silently drops unparseable entries.
    fn parse_known_peers(&self) -> Vec<grid_net::Multiaddr> {
        self.known_peers
            .iter()
            .filter_map(|s| {
                let addr: grid_net::Multiaddr = grid_net::strip_zx_multiaddr(s).parse().ok()?;
                Some(grid_net::sanitize_dial_addr(&addr))
            })
            .collect()
    }
}

const MAX_CACHED_PEERS: usize = 200;

/// Extract a (peer_id, ip) deduplication key from a multiaddr string.
fn peer_ip_key(s: &str) -> Option<(String, String)> {
    let stripped = grid_net::strip_zx_multiaddr(s);
    let addr: grid_net::Multiaddr = stripped.parse().ok()?;
    let ip = grid_net::extract_ip(&addr)?;
    let peer_id = grid_net::extract_peer_id(&addr)?;
    Some((peer_id.to_string(), ip))
}

/// Parse-sanitize-reserialize a peer multiaddr string, removing duplicate
/// `/p2p/` segments. Returns the original string unchanged if parsing fails.
fn sanitize_peer_string(s: &str) -> String {
    let stripped = grid_net::strip_zx_multiaddr(s);
    match stripped.parse::<grid_net::Multiaddr>() {
        Ok(addr) => grid_net::sanitize_dial_addr(&addr).to_string(),
        Err(_) => s.to_string(),
    }
}

/// Load peer multiaddrs from a JSON cache file (best-effort).
pub(crate) fn load_peer_cache(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str::<Vec<String>>(&data).ok())
        .unwrap_or_default()
}

/// Persist peer multiaddrs to a JSON cache file, capped at [`MAX_CACHED_PEERS`].
/// Sanitizes addresses before writing to prevent persisting malformed entries.
pub(crate) fn save_peer_cache(path: &Path, peers: &[String]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create peer cache dir: {e}"))?;
    }
    let sanitized: Vec<String> = peers.iter().map(|s| sanitize_peer_string(s)).collect();
    let capped: Vec<&String> = if sanitized.len() > MAX_CACHED_PEERS {
        sanitized[sanitized.len() - MAX_CACHED_PEERS..]
            .iter()
            .collect()
    } else {
        sanitized.iter().collect()
    };
    let json =
        serde_json::to_string_pretty(&capped).map_err(|e| format!("serialize peer cache: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write peer cache: {e}"))
}
