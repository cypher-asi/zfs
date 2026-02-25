use std::collections::HashSet;
use std::path::PathBuf;

use zfs_core::ProgramId;
use zfs_net::NetworkConfig;
use zfs_storage::StorageConfig;
use zfs_zode::{DefaultProgramsConfig, ZodeConfig};

pub(crate) struct Settings {
    pub data_dir: String,
    pub listen_addr: String,
    pub bootstrap_input: String,
    pub bootstrap_peers: Vec<String>,
    pub enable_zid: bool,
    pub enable_zchat: bool,
    pub topic_input: String,
    pub topics: Vec<String>,
    pub enable_kademlia: bool,
    pub kademlia_server_mode: bool,
    pub random_walk_interval_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: "zfs-zode-data".into(),
            listen_addr: "/ip4/127.0.0.1/udp/0/quic-v1".into(),
            bootstrap_input: String::new(),
            bootstrap_peers: Vec::new(),
            enable_zid: true,
            enable_zchat: true,
            topic_input: String::new(),
            topics: Vec::new(),
            enable_kademlia: true,
            kademlia_server_mode: true,
            random_walk_interval_secs: 30,
        }
    }
}

impl Settings {
    pub fn build_config(&self) -> Result<ZodeConfig, String> {
        let listen_addr: zfs_net::Multiaddr = self
            .listen_addr
            .parse()
            .map_err(|e| format!("Bad listen address: {e}"))?;

        let bootstrap = self.parse_bootstrap_peers()?;
        let topic_set = self.parse_topics()?;

        let kad_mode = if self.kademlia_server_mode {
            zfs_net::KademliaMode::Server
        } else {
            zfs_net::KademliaMode::Client
        };

        let discovery = zfs_net::DiscoveryConfig {
            enable_kademlia: self.enable_kademlia,
            kademlia_mode: kad_mode,
            random_walk_interval: std::time::Duration::from_secs(self.random_walk_interval_secs),
            ..Default::default()
        };

        let network = NetworkConfig::new(listen_addr)
            .with_bootstrap_peers(bootstrap)
            .with_discovery(discovery);
        let storage = StorageConfig::new(PathBuf::from(&self.data_dir));

        Ok(ZodeConfig {
            storage,
            default_programs: DefaultProgramsConfig {
                zid: self.enable_zid,
                zchat: self.enable_zchat,
            },
            topics: topic_set,
            sector_limits: Default::default(),
            network,
        })
    }

    fn parse_bootstrap_peers(&self) -> Result<Vec<zfs_net::Multiaddr>, String> {
        self.bootstrap_peers
            .iter()
            .map(|s| {
                zfs_net::strip_zx_multiaddr(s)
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
}
