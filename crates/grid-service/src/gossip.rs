use async_trait::async_trait;

/// Allows a service to intercept GossipSub messages on topics it owns.
///
/// The Zode dispatches incoming gossip to registered handlers before
/// falling through to the default `GossipSectorAppend` path.  A handler
/// that returns `true` from `handles_topic` receives the raw bytes; the
/// default handler is skipped for that message.
#[async_trait]
pub trait ServiceGossipHandler: Send + Sync + 'static {
    /// Return `true` if this handler should receive messages on `topic`.
    fn handles_topic(&self, topic: &str) -> bool;

    /// Called for every gossip message on a handled topic.
    ///
    /// `data` is the raw CBOR payload from GossipSub.
    /// `sender` is the formatted ZodeId of the message source, if known.
    async fn on_gossip(&self, topic: &str, data: &[u8], sender: Option<String>);
}
