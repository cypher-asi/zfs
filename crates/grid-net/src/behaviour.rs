use grid_core::{DirectMessage, DirectMessageAck, SectorRequest, SectorResponse};
use libp2p::swarm::NetworkBehaviour;

#[derive(NetworkBehaviour)]
pub(crate) struct GridBehaviour {
    pub(crate) connection_limits: libp2p::connection_limits::Behaviour,
    pub(crate) gossipsub: libp2p::gossipsub::Behaviour,
    pub(crate) sector_rr: libp2p::request_response::cbor::Behaviour<SectorRequest, SectorResponse>,
    pub(crate) direct_rr:
        libp2p::request_response::cbor::Behaviour<DirectMessage, DirectMessageAck>,
    pub(crate) kademlia: libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>,
    pub(crate) relay: libp2p::relay::client::Behaviour,
    pub(crate) identify: libp2p::identify::Behaviour,
    pub(crate) ping: libp2p::ping::Behaviour,
}
