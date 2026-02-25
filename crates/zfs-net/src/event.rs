use libp2p::request_response::{OutboundRequestId, ResponseChannel};
use libp2p::Multiaddr;
use zfs_core::{SectorRequest, SectorResponse};

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
}
