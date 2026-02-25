use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use libp2p::{gossipsub, kad, request_response, Multiaddr, PeerId, StreamProtocol};
use tracing::debug;

use crate::behaviour::ZfsBehaviour;
use crate::error::NetworkError;

const ZFS_SECTOR_PROTOCOL: &str = "/zfs/sector/1.0.0";
const ZFS_KAD_PROTOCOL: &str = "/zfs/kad/1.0.0";

pub(crate) fn build_swarm() -> Result<libp2p::Swarm<ZfsBehaviour>, NetworkError> {
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

    let swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|e| NetworkError::Transport(format!("{e}")))?
        .with_quic()
        .with_behaviour(|key| build_behaviour(key, gossipsub_config))
        .map_err(|e| NetworkError::Transport(format!("{e}")))?
        .build();
    Ok(swarm)
}

fn build_behaviour(
    key: &libp2p::identity::Keypair,
    gossipsub_config: gossipsub::Config,
) -> Result<ZfsBehaviour, Box<dyn std::error::Error + Send + Sync>> {
    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(key.clone()),
        gossipsub_config,
    )?;
    let sector_rr = request_response::cbor::Behaviour::new(
        [(
            StreamProtocol::new(ZFS_SECTOR_PROTOCOL),
            request_response::ProtocolSupport::Full,
        )],
        request_response::Config::default(),
    );
    let peer_id = key.public().to_peer_id();
    let mut kad_config = kad::Config::new(
        StreamProtocol::try_from_owned(ZFS_KAD_PROTOCOL.to_string()).expect("valid protocol name"),
    );
    kad_config.set_query_timeout(Duration::from_secs(60));
    let store = kad::store::MemoryStore::new(peer_id);
    let kademlia = kad::Behaviour::with_config(peer_id, store, kad_config);
    Ok(ZfsBehaviour {
        gossipsub,
        sector_rr,
        kademlia,
    })
}

pub(crate) fn dial_bootstrap_peers(
    swarm: &mut libp2p::Swarm<ZfsBehaviour>,
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

pub(crate) fn extract_peer_id(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter().find_map(|proto| match proto {
        libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    })
}
