use libp2p::swarm::NetworkBehaviour;
use zfs_core::{SectorRequest, SectorResponse};

#[derive(NetworkBehaviour)]
pub(crate) struct ZfsBehaviour {
    pub(crate) gossipsub: libp2p::gossipsub::Behaviour,
    pub(crate) sector_rr: libp2p::request_response::cbor::Behaviour<SectorRequest, SectorResponse>,
    pub(crate) kademlia: libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>,
}
