use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use libp2p::{
    connection_limits, gossipsub, identify, kad, ping, request_response, Multiaddr, PeerId,
    StreamProtocol,
};
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
        if let Some(ref peer) = message.source {
            peer.hash(&mut s);
        }
        message.sequence_number.hash(&mut s);
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
        .with_swarm_config(|cfg| cfg.with_idle_connection_timeout(Duration::from_secs(7200)))
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
        request_response::Config::default().with_request_timeout(Duration::from_secs(30)),
    );
    let peer_id = key.public().to_peer_id();
    // INVARIANT: GRID_KAD_PROTOCOL is a well-formed static protocol string.
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
    let ping = ping::Behaviour::new(ping::Config::new().with_interval(Duration::from_secs(15)));
    let connection_limits = connection_limits::Behaviour::new(
        connection_limits::ConnectionLimits::default()
            .with_max_established_incoming(Some(128))
            .with_max_established_outgoing(Some(128))
            .with_max_established_per_peer(Some(4))
            .with_max_pending_incoming(Some(64))
            .with_max_pending_outgoing(Some(64)),
    );
    Ok(GridBehaviour {
        connection_limits,
        gossipsub,
        sector_rr,
        kademlia,
        relay,
        identify,
        ping,
    })
}

/// Max addresses to dial per peer during bootstrap.  Avoids hammering
/// stale NAT-mapped ports that have accumulated in the peer cache.
const MAX_BOOTSTRAP_ADDRS_PER_PEER: usize = 2;

pub(crate) fn dial_bootstrap_peers(
    swarm: &mut libp2p::Swarm<GridBehaviour>,
    peers: &[Multiaddr],
    kademlia_enabled: bool,
) {
    let local_peer_id = *swarm.local_peer_id();

    let mut by_peer: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
    let mut no_peer: Vec<Multiaddr> = Vec::new();

    for peer_addr in peers {
        let normalized = crate::addr::normalize_multiaddr(peer_addr);
        if !crate::addr::has_transport(&normalized) {
            debug!(%peer_addr, "skipping bootstrap peer with no transport address");
            continue;
        }
        if let Some(peer_id) = crate::addr::extract_peer_id(peer_addr) {
            if peer_id == local_peer_id {
                debug!(%peer_addr, "skipping self-dial");
                continue;
            }
            if kademlia_enabled && crate::addr::is_globally_routable(&normalized) {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, normalized);
            }
            by_peer.entry(peer_id).or_default().push(peer_addr.clone());
        } else {
            no_peer.push(peer_addr.clone());
        }
    }

    for (peer_id, addrs) in &by_peer {
        let to_dial = if addrs.len() > MAX_BOOTSTRAP_ADDRS_PER_PEER {
            &addrs[addrs.len() - MAX_BOOTSTRAP_ADDRS_PER_PEER..]
        } else {
            addrs.as_slice()
        };
        debug!(%peer_id, total = addrs.len(), dialing = to_dial.len(), "bootstrap peer");
        for addr in to_dial {
            match swarm.dial(addr.clone()) {
                Ok(()) => debug!(%addr, "dialed bootstrap peer"),
                Err(e) => debug!(%addr, error = %e, "failed to dial bootstrap peer"),
            }
        }
    }

    for addr in &no_peer {
        match swarm.dial(addr.clone()) {
            Ok(()) => debug!(%addr, "dialed bootstrap peer (no peer id)"),
            Err(e) => debug!(%addr, error = %e, "failed to dial bootstrap peer"),
        }
    }
}

pub(crate) fn dial_relay_peers(
    swarm: &mut libp2p::Swarm<GridBehaviour>,
    peers: &[Multiaddr],
    kademlia_enabled: bool,
) {
    for relay_addr in peers {
        if kademlia_enabled {
            if let Some(peer_id) = crate::addr::extract_peer_id(relay_addr) {
                let normalized = crate::addr::normalize_multiaddr(relay_addr);
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, normalized);
                debug!(%peer_id, %relay_addr, "added relay peer to kademlia");
            }
        }
        match swarm.dial(relay_addr.clone()) {
            Ok(()) => debug!(%relay_addr, "dialed relay peer"),
            Err(e) => debug!(%relay_addr, error = %e, "failed to dial relay peer"),
        }
    }
}
