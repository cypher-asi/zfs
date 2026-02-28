use std::time::Duration;

use libp2p::Multiaddr;

/// Configuration for the Grid network layer.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Address to listen on (e.g. `/ip4/0.0.0.0/udp/3690/quic-v1`).
    pub listen_addr: Multiaddr,

    /// Bootstrap peers to connect to on startup.
    pub bootstrap_peers: Vec<Multiaddr>,

    /// Discovery settings (Kademlia DHT, mDNS).
    pub discovery: DiscoveryConfig,

    /// Relay transport settings.
    pub relay: RelayConfig,

    /// Pre-existing libp2p keypair. When `Some`, the swarm reuses this
    /// identity instead of generating a fresh one each launch.
    pub keypair: Option<libp2p::identity::Keypair>,
}

/// Discovery configuration for Kademlia DHT and mDNS.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Enable Kademlia DHT for automatic peer discovery.
    /// Default: `true`.
    pub enable_kademlia: bool,

    /// Kademlia mode. Zodes should use `Server`; SDK clients should use `Client`.
    /// Default: `Server`.
    pub kademlia_mode: KademliaMode,

    /// Interval between random walk queries to discover new peers.
    /// Default: 30 seconds.
    pub random_walk_interval: Duration,

    /// Maximum number of concurrent outbound dials triggered by discovery.
    /// Default: 8.
    pub max_concurrent_discovery_dials: usize,

    /// How long to suppress re-dialing a peer after a failed connection attempt.
    /// Prevents hammering unreachable peers with stale NAT-mapped addresses.
    /// Default: 5 minutes.
    pub dial_backoff_duration: Duration,
}

/// Kademlia operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KademliaMode {
    /// This Zode responds to DHT queries from other peers (long-lived Zodes).
    Server,
    /// This Zode queries the DHT but does not serve routing info (short-lived SDK clients).
    Client,
}

/// The well-known relay peer that all nodes connect to by default.
pub const DEFAULT_RELAY_PEER: &str =
    "/ip4/3.129.15.45/tcp/3691/p2p/12D3KooWHvyFJm77ZAUR7DzAhRCjyhGgcwNxhQAoCptScBhQCs2b";

/// Relay transport configuration.
#[derive(Debug, Clone, Default)]
pub struct RelayConfig {
    /// Enable relay transport support.
    pub enabled: bool,
    /// Relay peers to dial on startup.
    pub relay_peers: Vec<Multiaddr>,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enable_kademlia: true,
            kademlia_mode: KademliaMode::Server,
            random_walk_interval: Duration::from_secs(30),
            max_concurrent_discovery_dials: 8,
            dial_backoff_duration: Duration::from_secs(300),
        }
    }
}

impl NetworkConfig {
    pub fn new(listen_addr: Multiaddr) -> Self {
        Self {
            listen_addr,
            bootstrap_peers: Vec::new(),
            discovery: DiscoveryConfig::default(),
            relay: RelayConfig::default(),
            keypair: None,
        }
    }

    pub fn with_bootstrap_peers(mut self, peers: Vec<Multiaddr>) -> Self {
        self.bootstrap_peers = peers;
        self
    }

    pub fn with_discovery(mut self, discovery: DiscoveryConfig) -> Self {
        self.discovery = discovery;
        self
    }

    pub fn with_relay(mut self, relay: RelayConfig) -> Self {
        self.relay = relay;
        self
    }

    pub fn with_keypair(mut self, keypair: libp2p::identity::Keypair) -> Self {
        self.keypair = Some(keypair);
        self
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addr: "/ip4/0.0.0.0/udp/3690/quic-v1"
                .parse()
                .expect("well-known constant multiaddr"),
            bootstrap_peers: Vec::new(),
            discovery: DiscoveryConfig::default(),
            relay: RelayConfig::default(),
            keypair: None,
        }
    }
}
