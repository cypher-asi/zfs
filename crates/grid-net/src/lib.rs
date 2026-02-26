#![forbid(unsafe_code)]
//! Grid network abstraction over libp2p.
//!
//! Provides QUIC transport, GossipSub topic subscription, and a
//! request-response protocol for sector storage exchanges.
//!
//! This is the **only** crate in the workspace that depends on libp2p.
//!
//! # Usage
//!
//! ```rust,no_run
//! # async fn example() -> Result<(), grid_net::NetworkError> {
//! let mut svc = grid_net::NetworkService::new(Default::default()).await?;
//! svc.subscribe("prog/abc123")?;
//! loop {
//!     if let Some(event) = svc.next_event().await {
//!         // handle event
//!     }
//! }
//! # }
//! ```

mod behaviour;
mod builder;
mod config;
mod error;
mod event;
mod service;

pub use config::{DiscoveryConfig, KademliaMode, NetworkConfig};
pub use error::NetworkError;
pub use event::NetworkEvent;
pub use service::NetworkService;

// Re-export libp2p types that appear in the public API so consumers
// do not need a direct libp2p dependency.
pub use libp2p::identity::Keypair;
pub use libp2p::request_response::{OutboundRequestId, ResponseChannel};
pub use libp2p::Multiaddr;

/// A Zode's identity on the network (wraps libp2p `PeerId`).
pub type ZodeId = libp2p::PeerId;

/// The human-readable prefix for Zode addresses.
const ZODE_ID_PREFIX: &str = "Zx";

/// Format a [`ZodeId`] for display with the canonical `Zx` prefix.
///
/// On the wire and in storage the raw `PeerId` bytes are used; this
/// prefix is **display-only**.
pub fn format_zode_id(id: &ZodeId) -> String {
    format!("{ZODE_ID_PREFIX}{id}")
}

/// Parse a `Zx`-prefixed Zode address back into a [`ZodeId`].
///
/// Accepts both `Zx`-prefixed and bare `PeerId` strings for
/// backwards-compatibility.
pub fn parse_zode_id(s: &str) -> Result<ZodeId, libp2p::identity::ParseError> {
    let raw = s.strip_prefix(ZODE_ID_PREFIX).unwrap_or(s);
    raw.parse()
}

/// Strip the display-only `Zx` prefix from the `/p2p/<peer_id>` component of a
/// multiaddr string so libp2p can parse the raw `PeerId`.
///
/// Returns the input unchanged when no `Zx` prefix is present.
pub fn strip_zx_multiaddr(addr: &str) -> std::borrow::Cow<'_, str> {
    if let Some(idx) = addr.find("/p2p/Zx") {
        let prefix_start = idx + "/p2p/".len();
        let mut out = String::with_capacity(addr.len() - 2);
        out.push_str(&addr[..prefix_start]);
        out.push_str(&addr[prefix_start + 2..]);
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(addr)
    }
}
