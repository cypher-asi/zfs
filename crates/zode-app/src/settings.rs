use std::collections::HashSet;
use std::path::PathBuf;

use grid_core::ProgramId;
use grid_net::NetworkConfig;
use grid_storage::StorageConfig;
use zode::{DefaultProgramsConfig, RpcConfig, ZodeConfig};

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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: ".zode/data".into(),
            listen_addr: "/ip4/127.0.0.1/udp/3690/quic-v1".into(),
            bootstrap_input: String::new(),
            bootstrap_peers: Vec::new(),
            enable_relay: false,
            relay_input: String::new(),
            relay_peers: Vec::new(),
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
        }
    }
}

impl Settings {
    pub fn build_config(&self) -> Result<ZodeConfig, String> {
        let listen_addr: grid_net::Multiaddr = self
            .listen_addr
            .parse()
            .map_err(|e| format!("Bad listen address: {e}"))?;

        let bootstrap = self.parse_bootstrap_peers()?;
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
}
