use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use libp2p::{gossipsub, identify, kad, request_response, Multiaddr, PeerId, StreamProtocol};
use tracing::debug;

use crate::behaviour::GridBehaviour;
use crate::error::NetworkError;

const GRID_SECTOR_PROTOCOL: &str = "/grid/sector/1.0.0";
const GRID_KAD_PROTOCOL: &str = "/grid/kad/1.0.0";
const GRID_IDENTIFY_PROTOCOL: &str = "/grid/id/1.0.0";

pub(crate) fn build_swarm(
    keypair: Option<libp2p::identity::Keypair>,
) -> Result<(libp2p::Swarm<GridBehaviour>, libp2p::identity::Keypair), NetworkError> {
    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        gossipsub::MessageId::from(s.finish().to_string())
    };

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(10))
        .validation_mode(gossipsub::ValidationMode::Permissive)
        .message_id_fn(message_id_fn)
        .build()
        .map_err(|e| NetworkError::Config(format!("{e}")))?;

    let kp = keypair.unwrap_or_else(libp2p::identity::Keypair::generate_ed25519);

    let swarm = libp2p::SwarmBuilder::with_existing_identity(kp.clone())
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|e| NetworkError::Transport(format!("{e}")))?
        .with_quic()
        .with_relay_client(libp2p::noise::Config::new, libp2p::yamux::Config::default)
        .map_err(|e| NetworkError::Transport(format!("{e}")))?
        .with_behaviour(|key, relay| build_behaviour(key, gossipsub_config, relay))
        .map_err(|e| NetworkError::Transport(format!("{e}")))?
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();
    Ok((swarm, kp))
}

fn build_behaviour(
    key: &libp2p::identity::Keypair,
    gossipsub_config: gossipsub::Config,
    relay: libp2p::relay::client::Behaviour,
) -> Result<GridBehaviour, Box<dyn std::error::Error + Send + Sync>> {
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(key.clone()),
        gossipsub_config,
    )?;
    let sector_rr = request_response::cbor::Behaviour::new(
        [(
            StreamProtocol::new(GRID_SECTOR_PROTOCOL),
            request_response::ProtocolSupport::Full,
        )],
        request_response::Config::default(),
    );
    let peer_id = key.public().to_peer_id();
    let mut kad_config = kad::Config::new(
        StreamProtocol::try_from_owned(GRID_KAD_PROTOCOL.to_string()).expect("valid protocol name"),
    );
    kad_config.set_query_timeout(Duration::from_secs(60));
    let store = kad::store::MemoryStore::new(peer_id);
    let kademlia = kad::Behaviour::with_config(peer_id, store, kad_config);
    let identify = identify::Behaviour::new(
        identify::Config::new(GRID_IDENTIFY_PROTOCOL.to_string(), key.public())
            .with_push_listen_addr_updates(true),
    );
    Ok(GridBehaviour {
        gossipsub,
        sector_rr,
        kademlia,
        relay,
        identify,
    })
}

pub(crate) fn dial_bootstrap_peers(
    swarm: &mut libp2p::Swarm<GridBehaviour>,
    peers: &[Multiaddr],
    kademlia_enabled: bool,
) -> Result<(), NetworkError> {
    for peer_addr in peers {
        if kademlia_enabled {
            if let Some(peer_id) = extract_peer_id(peer_addr) {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, peer_addr.clone());
                debug!(%peer_id, %peer_addr, "added bootstrap peer to kademlia");
            }
        }
        swarm
            .dial(peer_addr.clone())
            .map_err(|e| NetworkError::Dial(e.to_string()))?;
    }
    Ok(())
}

pub(crate) fn dial_relay_peers(
    swarm: &mut libp2p::Swarm<GridBehaviour>,
    peers: &[Multiaddr],
    kademlia_enabled: bool,
) {
    for relay_addr in peers {
        if kademlia_enabled {
            if let Some(peer_id) = extract_peer_id(relay_addr) {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, relay_addr.clone());
                debug!(%peer_id, %relay_addr, "added relay peer to kademlia");
            }
        }
        match swarm.dial(relay_addr.clone()) {
            Ok(()) => debug!(%relay_addr, "dialed relay peer"),
            Err(e) => debug!(%relay_addr, error = %e, "failed to dial relay peer"),
        }
    }
}

pub(crate) fn extract_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter().find_map(|proto| match proto {
        libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    })
}
