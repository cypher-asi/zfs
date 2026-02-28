use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt;
use libp2p::{gossipsub, identify, kad, request_response, swarm::SwarmEvent, Multiaddr, PeerId};
use tracing::{debug, info, warn};

use crate::behaviour::{GridBehaviour, GridBehaviourEvent};
use crate::builder::{build_swarm, dial_bootstrap_peers, dial_relay_peers, extract_peer_id};
use crate::config::{KademliaMode, NetworkConfig};
use crate::error::NetworkError;
use crate::event::NetworkEvent;

/// Manages the libp2p swarm and exposes the Grid network API.
///
/// The caller drives the service by calling [`next_event`](Self::next_event)
/// in a loop, handling events and issuing commands via the other methods.
pub struct NetworkService {
    swarm: libp2p::Swarm<GridBehaviour>,
    keypair: libp2p::identity::Keypair,
    kademlia_enabled: bool,
    kademlia_bootstrapped: bool,
    random_walk_interval: Duration,
    max_discovery_dials: usize,
    pending_discovery_dials: usize,
    discovered_peers: HashSet<PeerId>,
    /// Observed addresses for peers (from connection endpoints and Kademlia).
    peer_addresses: HashMap<PeerId, Vec<Multiaddr>>,
    /// Raw relay transport addresses (without /p2p/ or /p2p-circuit), used
    /// to recognise when a newly-established connection is to a relay so we
    /// can start a circuit listener on it.
    relay_transport_addrs: Vec<Multiaddr>,
    /// Relay circuit addresses we have already started listening on.
    active_relay_listeners: HashSet<PeerId>,
    /// Pre-computed relay circuit base addresses derived from relay config,
    /// used to speculatively construct circuit-routed paths for discovered peers.
    relay_circuit_bases: Vec<Multiaddr>,
}

impl NetworkService {
    /// Create and start the network service.
    ///
    /// Begins listening on `config.listen_addr` and dials any bootstrap peers.
    /// When `config.discovery.enable_kademlia` is true, seeds the Kademlia
    /// routing table and triggers the initial bootstrap.
    pub async fn new(config: NetworkConfig) -> Result<Self, NetworkError> {
        let kademlia_enabled = config.discovery.enable_kademlia;
        let random_walk_interval = config.discovery.random_walk_interval;
        let max_discovery_dials = config.discovery.max_concurrent_discovery_dials;
        let kademlia_mode = config.discovery.kademlia_mode;
        let relay_enabled = config.relay.enabled;

        let (mut swarm, keypair) = build_swarm(config.keypair)?;

        if kademlia_enabled {
            let mode = match kademlia_mode {
                KademliaMode::Server => kad::Mode::Server,
                KademliaMode::Client => kad::Mode::Client,
            };
            swarm.behaviour_mut().kademlia.set_mode(Some(mode));
        }

        swarm
            .listen_on(config.listen_addr)
            .map_err(|e| NetworkError::Transport(e.to_string()))?;

        dial_bootstrap_peers(&mut swarm, &config.bootstrap_peers, kademlia_enabled)?;

        let mut relay_transport_addrs = Vec::new();
        if relay_enabled {
            for relay_addr in &config.relay.relay_peers {
                relay_transport_addrs.push(strip_p2p(relay_addr));
            }
            dial_relay_peers(&mut swarm, &config.relay.relay_peers, kademlia_enabled);
            debug!(
                count = config.relay.relay_peers.len(),
                "relay dialing configured"
            );
        }

        let relay_circuit_bases: Vec<Multiaddr> = if relay_enabled {
            config
                .relay
                .relay_peers
                .iter()
                .filter_map(|addr| {
                    let peer_id = extract_peer_id(addr)?;
                    let transport = strip_p2p(addr);
                    Some(
                        transport
                            .with(libp2p::multiaddr::Protocol::P2p(peer_id))
                            .with(libp2p::multiaddr::Protocol::P2pCircuit),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut kademlia_bootstrapped = false;
        if kademlia_enabled {
            match swarm.behaviour_mut().kademlia.bootstrap() {
                Ok(_) => {
                    kademlia_bootstrapped = true;
                }
                Err(e) => {
                    debug!("kademlia bootstrap deferred until first connection: {e:?}");
                }
            }
        }

        Ok(Self {
            swarm,
            keypair,
            kademlia_enabled,
            kademlia_bootstrapped,
            random_walk_interval,
            max_discovery_dials,
            pending_discovery_dials: 0,
            discovered_peers: HashSet::new(),
            peer_addresses: HashMap::new(),
            relay_transport_addrs,
            active_relay_listeners: HashSet::new(),
            relay_circuit_bases,
        })
    }

    /// The local Zode ID.
    pub fn local_zode_id(&self) -> &PeerId {
        self.swarm.local_peer_id()
    }

    /// The libp2p keypair as protobuf-encoded bytes for vault persistence.
    pub fn keypair_to_protobuf(&self) -> Vec<u8> {
        self.keypair
            .to_protobuf_encoding()
            .expect("ed25519 keypair protobuf encoding cannot fail")
    }

    /// Subscribe to a GossipSub topic (e.g. `"prog/{program_id_hex}"`).
    pub fn subscribe(&mut self, topic: &str) -> Result<(), NetworkError> {
        let topic = gossipsub::IdentTopic::new(topic);
        self.swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&topic)
            .map_err(|e| NetworkError::Subscription(e.to_string()))?;
        Ok(())
    }

    /// Unsubscribe from a GossipSub topic.
    pub fn unsubscribe(&mut self, topic: &str) -> Result<(), NetworkError> {
        let topic = gossipsub::IdentTopic::new(topic);
        if !self.swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
            return Err(NetworkError::Subscription("not subscribed to topic".into()));
        }
        Ok(())
    }

    /// Publish data to a GossipSub topic.
    pub fn publish(&mut self, topic: &str, data: Vec<u8>) -> Result<(), NetworkError> {
        let topic = gossipsub::IdentTopic::new(topic);
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic, data)
            .map_err(|e| NetworkError::Publish(e.to_string()))?;
        Ok(())
    }

    /// Send a sector request to a specific peer.
    pub fn send_sector_request(
        &mut self,
        peer: &PeerId,
        request: grid_core::SectorRequest,
    ) -> request_response::OutboundRequestId {
        self.swarm
            .behaviour_mut()
            .sector_rr
            .send_request(peer, request)
    }

    /// Send a sector response on a previously received request channel.
    pub fn send_sector_response(
        &mut self,
        channel: request_response::ResponseChannel<grid_core::SectorResponse>,
        response: grid_core::SectorResponse,
    ) -> Result<(), NetworkError> {
        self.swarm
            .behaviour_mut()
            .sector_rr
            .send_response(channel, response)
            .map_err(|_| NetworkError::ResponseFailed)
    }

    /// Returns the list of currently connected peer IDs.
    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.swarm.connected_peers().copied().collect()
    }

    /// Returns connected peers paired with their known addresses.
    /// Useful for persisting peer info so the node can reconnect on
    /// next startup without full re-discovery.
    pub fn connected_peers_with_addrs(&self) -> Vec<(PeerId, Vec<Multiaddr>)> {
        self.swarm
            .connected_peers()
            .map(|peer| {
                let addrs = self
                    .peer_addresses
                    .get(peer)
                    .cloned()
                    .unwrap_or_default();
                (*peer, addrs)
            })
            .collect()
    }

    /// Returns all peer addresses observed during this session regardless
    /// of current connection status. Safe to call after shutdown when the
    /// swarm may have already closed connections.
    pub fn all_known_peer_addrs(&self) -> Vec<(PeerId, Vec<Multiaddr>)> {
        self.peer_addresses
            .iter()
            .map(|(peer, addrs)| (*peer, addrs.clone()))
            .collect()
    }

    /// Returns all observed peer addresses as dial-ready multiaddr strings
    /// (each ending with exactly one `/p2p/<peer_id>` suffix).
    ///
    /// Relay circuit addresses already embed the destination peer ID
    /// (e.g. `.../p2p-circuit/p2p/<peer>`), so blindly appending would
    /// produce a malformed doubled suffix. This method normalises that.
    pub fn peer_multiaddr_strings(&self) -> Vec<String> {
        self.peer_addresses
            .iter()
            .flat_map(|(peer, addrs)| {
                let peer = *peer;
                addrs.iter().map(move |a| {
                    let already_ends_with_peer = a
                        .iter()
                        .last()
                        .is_some_and(|p| p == libp2p::multiaddr::Protocol::P2p(peer));
                    if already_ends_with_peer {
                        a.to_string()
                    } else {
                        format!("{a}/p2p/{peer}")
                    }
                })
            })
            .collect()
    }

    /// Dial a peer at the given multiaddr.
    pub fn dial(&mut self, addr: Multiaddr) -> Result<(), NetworkError> {
        self.swarm
            .dial(addr)
            .map_err(|e| NetworkError::Dial(e.to_string()))
    }

    /// Drive the swarm event loop, returning the next high-level network event.
    ///
    /// Must be called in a loop to keep the network alive. When Kademlia is
    /// enabled, a periodic random walk timer fires between swarm events.
    pub async fn next_event(&mut self) -> Option<NetworkEvent> {
        let sleep = tokio::time::sleep(self.random_walk_interval);
        tokio::pin!(sleep);

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    if let Some(net_event) = self.handle_swarm_event(event) {
                        return Some(net_event);
                    }
                }
                () = &mut sleep, if self.kademlia_enabled => {
                    self.trigger_random_walk();
                    sleep.as_mut().reset(tokio::time::Instant::now() + self.random_walk_interval);
                }
            }
        }
    }

    fn handle_swarm_event(
        &mut self,
        event: SwarmEvent<GridBehaviourEvent>,
    ) -> Option<NetworkEvent> {
        match event {
            SwarmEvent::Behaviour(event) => self.map_behaviour_event(event),
            SwarmEvent::ConnectionEstablished {
                peer_id,
                num_established,
                endpoint,
                ..
            } => {
                if self.pending_discovery_dials > 0 {
                    self.pending_discovery_dials -= 1;
                }
                debug!(%peer_id, num = %num_established, "connection established");
                let addr = endpoint.get_remote_address().clone();
                self.peer_addresses
                    .entry(peer_id)
                    .or_default()
                    .push(addr.clone());
                if self.kademlia_enabled {
                    self.swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, addr.clone());
                }

                self.try_start_relay_listeners(&peer_id, &addr);
                self.try_kademlia_bootstrap();

                (num_established.get() == 1).then(|| NetworkEvent::PeerConnected(peer_id))
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                num_established,
                ..
            } => {
                if self.pending_discovery_dials > 0 {
                    self.pending_discovery_dials -= 1;
                }
                debug!(%peer_id, num = %num_established, "connection closed");
                if num_established == 0 {
                    self.active_relay_listeners.remove(&peer_id);
                }
                (num_established == 0).then(|| NetworkEvent::PeerDisconnected(peer_id))
            }
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(%address, "listening");
                Some(NetworkEvent::ListenAddress(address))
            }
            _ => None,
        }
    }

    /// When we connect to a relay peer, start listening on its circuit address
    /// so other zodes can reach us through the relay.
    fn try_start_relay_listeners(&mut self, connected_peer: &PeerId, remote_addr: &Multiaddr) {
        if self.active_relay_listeners.contains(connected_peer) {
            return;
        }

        let remote_transport = strip_p2p(remote_addr);
        let is_relay = self
            .relay_transport_addrs
            .iter()
            .any(|ra| *ra == remote_transport);

        if !is_relay {
            return;
        }

        let circuit_addr = remote_transport
            .with(libp2p::multiaddr::Protocol::P2p(*connected_peer))
            .with(libp2p::multiaddr::Protocol::P2pCircuit);

        match self.swarm.listen_on(circuit_addr.clone()) {
            Ok(_) => {
                info!(%circuit_addr, "listening via relay circuit");
                self.active_relay_listeners.insert(*connected_peer);
            }
            Err(e) => {
                warn!(%circuit_addr, error = %e, "failed to listen on relay circuit");
            }
        }
    }

    /// Retry Kademlia bootstrap if it hasn't succeeded yet.
    fn try_kademlia_bootstrap(&mut self) {
        if !self.kademlia_enabled || self.kademlia_bootstrapped {
            return;
        }
        match self.swarm.behaviour_mut().kademlia.bootstrap() {
            Ok(_) => {
                self.kademlia_bootstrapped = true;
                info!("kademlia bootstrap started");
            }
            Err(e) => {
                debug!("kademlia bootstrap still waiting for peers: {e:?}");
            }
        }
    }

    /// Issue a random walk query to discover new peers.
    fn trigger_random_walk(&mut self) {
        let random_peer = PeerId::random();
        self.swarm
            .behaviour_mut()
            .kademlia
            .get_closest_peers(random_peer);
        debug!("kademlia random walk triggered");
    }

    /// Try to auto-dial a newly discovered peer (respects concurrency limit).
    ///
    /// Always dials by `PeerId` so libp2p tries all known addresses
    /// (direct, relay circuit, etc.) rather than just the first one.
    fn try_discovery_dial(&mut self, peer_id: &PeerId, addrs: &[Multiaddr]) {
        if self.swarm.is_connected(peer_id) {
            return;
        }
        if *peer_id == *self.swarm.local_peer_id() {
            return;
        }
        if self.pending_discovery_dials >= self.max_discovery_dials {
            debug!(%peer_id, "skipping discovery dial (concurrency limit)");
            return;
        }

        for addr in addrs {
            self.swarm
                .behaviour_mut()
                .kademlia
                .add_address(peer_id, addr.clone());
        }

        for base in &self.relay_circuit_bases {
            let via_relay = base
                .clone()
                .with(libp2p::multiaddr::Protocol::P2p(*peer_id));
            self.swarm
                .behaviour_mut()
                .kademlia
                .add_address(peer_id, via_relay);
        }

        debug!(%peer_id, num_addrs = addrs.len(), "auto-dialing discovered peer");
        match self.swarm.dial(*peer_id) {
            Ok(()) => {
                self.pending_discovery_dials += 1;
            }
            Err(e) => {
                debug!(%peer_id, error = %e, "failed to auto-dial discovered peer");
            }
        }
    }

    fn map_behaviour_event(&mut self, event: GridBehaviourEvent) -> Option<NetworkEvent> {
        match event {
            GridBehaviourEvent::Gossipsub(ev) => Self::map_gossip_event(ev),
            GridBehaviourEvent::SectorRr(ev) => Self::map_sector_rr_event(ev),
            GridBehaviourEvent::Kademlia(ev) => self.map_kademlia_event(ev),
            GridBehaviourEvent::Relay(ev) => {
                debug!(?ev, "relay event");
                None
            }
            GridBehaviourEvent::Identify(ev) => self.map_identify_event(ev),
        }
    }

    fn map_identify_event(&mut self, event: identify::Event) -> Option<NetworkEvent> {
        match event {
            identify::Event::Received { peer_id, info, .. } => {
                let listen_addrs = self.ingest_identify_info(
                    peer_id,
                    info,
                    "identify received",
                );
                if self.discovered_peers.insert(peer_id) {
                    self.try_discovery_dial(&peer_id, &listen_addrs);
                    Some(NetworkEvent::PeerDiscovered {
                        zode_id: peer_id,
                        addresses: listen_addrs,
                    })
                } else {
                    None
                }
            }
            identify::Event::Sent { peer_id, .. } => {
                debug!(%peer_id, "identify sent");
                None
            }
            identify::Event::Error { peer_id, error, .. } => {
                debug!(%peer_id, %error, "identify error");
                None
            }
            identify::Event::Pushed { peer_id, info, .. } => {
                let _ = self.ingest_identify_info(peer_id, info, "identify pushed");
                None
            }
        }
    }

    fn ingest_identify_info(
        &mut self,
        peer_id: PeerId,
        info: identify::Info,
        log_message: &str,
    ) -> Vec<Multiaddr> {
        let listen_addrs = info.listen_addrs.clone();
        debug!(
            %peer_id,
            protocols = ?info.protocols,
            listen_addrs = ?listen_addrs,
            observed = %info.observed_addr,
            "{log_message}"
        );

        self.swarm.add_external_address(info.observed_addr);

        if self.kademlia_enabled {
            for addr in &listen_addrs {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, addr.clone());
            }
        }

        let stored = self.peer_addresses.entry(peer_id).or_default();
        for a in &listen_addrs {
            if !stored.contains(a) {
                stored.push(a.clone());
            }
        }

        listen_addrs
    }

    fn map_gossip_event(event: gossipsub::Event) -> Option<NetworkEvent> {
        match event {
            gossipsub::Event::Message { message, .. } => Some(NetworkEvent::GossipMessage {
                source: message.source,
                topic: message.topic.to_string(),
                data: message.data,
            }),
            _ => None,
        }
    }

    fn map_sector_rr_event(
        event: request_response::Event<grid_core::SectorRequest, grid_core::SectorResponse>,
    ) -> Option<NetworkEvent> {
        match event {
            request_response::Event::Message { peer, message, .. } => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => Some(NetworkEvent::IncomingSectorRequest {
                    peer,
                    request: Box::new(request),
                    channel,
                }),
                request_response::Message::Response {
                    request_id,
                    response,
                } => Some(NetworkEvent::SectorRequestResult {
                    peer,
                    request_id,
                    response: Box::new(response),
                }),
            },
            request_response::Event::OutboundFailure {
                peer,
                request_id,
                error,
                ..
            } => Some(NetworkEvent::SectorOutboundFailure {
                peer,
                request_id,
                error: error.to_string(),
            }),
            _ => None,
        }
    }

    fn map_kademlia_event(&mut self, event: kad::Event) -> Option<NetworkEvent> {
        match event {
            kad::Event::RoutingUpdated {
                peer, addresses, ..
            } => {
                let addrs: Vec<Multiaddr> = addresses.iter().cloned().collect();
                let stored = self.peer_addresses.entry(peer).or_default();
                for a in &addrs {
                    if !stored.contains(a) {
                        stored.push(a.clone());
                    }
                }
                if self.discovered_peers.insert(peer) {
                    self.try_discovery_dial(&peer, &addrs);
                    Some(NetworkEvent::PeerDiscovered {
                        zode_id: peer,
                        addresses: addrs,
                    })
                } else {
                    None
                }
            }
            kad::Event::OutboundQueryProgressed {
                result: kad::QueryResult::GetClosestPeers(Ok(ok)),
                ..
            } => self.handle_closest_peers(ok),
            kad::Event::OutboundQueryProgressed {
                result: kad::QueryResult::Bootstrap(Ok(result)),
                ..
            } => {
                debug!(
                    num_remaining = result.num_remaining,
                    "kademlia bootstrap progressed"
                );
                None
            }
            other => {
                debug!(?other, "kademlia event");
                None
            }
        }
    }

    fn handle_closest_peers(&mut self, ok: kad::GetClosestPeersOk) -> Option<NetworkEvent> {
        let mut first_new_peer = None;
        for peer in &ok.peers {
            let peer_id = peer.peer_id;
            let addrs: Vec<Multiaddr> = peer.addrs.clone();
            let stored = self.peer_addresses.entry(peer_id).or_default();
            for a in &addrs {
                if !stored.contains(a) {
                    stored.push(a.clone());
                }
            }
            if self.discovered_peers.insert(peer_id) {
                self.try_discovery_dial(&peer_id, &addrs);
                if first_new_peer.is_none() {
                    first_new_peer = Some(NetworkEvent::PeerDiscovered {
                        zode_id: peer_id,
                        addresses: addrs,
                    });
                }
            }
        }
        first_new_peer
    }
}

/// Strip any trailing `/p2p/<peer_id>` component from a multiaddr so only
/// the transport portion remains. Used to compare relay addresses regardless
/// of whether they were configured with or without an explicit peer ID.
fn strip_p2p(addr: &Multiaddr) -> Multiaddr {
    addr.iter()
        .filter(|p| !matches!(p, libp2p::multiaddr::Protocol::P2p(_)))
        .collect()
}
