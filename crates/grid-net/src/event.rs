use grid_core::{DirectMessage, DirectMessageAck, SectorRequest, SectorResponse};
use libp2p::request_response::{OutboundRequestId, ResponseChannel};
use libp2p::Multiaddr;

use crate::ZodeId;

/// Events produced by the [`NetworkService`](crate::NetworkService) event loop.
pub enum NetworkEvent {
    /// A new Zode connection was established.
    PeerConnected(ZodeId),

    /// A Zode connection was closed.
    PeerDisconnected(ZodeId),

    /// A new Zode was discovered via DHT or mDNS (not yet necessarily connected).
    PeerDiscovered {
        zode_id: ZodeId,
        addresses: Vec<Multiaddr>,
    },

    /// An incoming sector request from a remote peer.
    IncomingSectorRequest {
        peer: ZodeId,
        request: Box<SectorRequest>,
        channel: ResponseChannel<SectorResponse>,
    },

    /// Response received for an outbound sector request.
    SectorRequestResult {
        peer: ZodeId,
        request_id: OutboundRequestId,
        response: Box<SectorResponse>,
    },

    /// An outbound sector request failed.
    SectorOutboundFailure {
        peer: ZodeId,
        request_id: OutboundRequestId,
        error: String,
    },

    /// A GossipSub message received on a subscribed topic.
    GossipMessage {
        source: Option<ZodeId>,
        topic: String,
        data: Vec<u8>,
    },

    /// A new listen address was established.
    ListenAddress(Multiaddr),

    /// The relay circuit listener started successfully.
    RelayListening { circuit_addr: Multiaddr },

    /// The relay circuit listener failed to start.
    RelayFailed {
        circuit_addr: Multiaddr,
        error: String,
    },

    /// An outgoing dial/connection attempt failed.
    ConnectionFailed { peer: Option<ZodeId>, error: String },

    /// Kademlia DHT bootstrap started successfully.
    KademliaBootstrapped,

    /// An incoming direct message from a peer.
    IncomingDirectMessage {
        peer: ZodeId,
        message: DirectMessage,
        channel: ResponseChannel<DirectMessageAck>,
    },

    /// Response for an outbound direct message.
    DirectMessageResult {
        peer: ZodeId,
        request_id: OutboundRequestId,
        response: DirectMessageAck,
    },

    /// An outbound direct message failed.
    DirectMessageFailure {
        peer: ZodeId,
        request_id: OutboundRequestId,
        error: String,
    },
}
