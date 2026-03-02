use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use grid_programs_zephyr::{ZephyrGlobalMessage, ZephyrZoneMessage};
use grid_service::ServiceGossipHandler;
use tracing::{debug, warn};

/// Gossip handler for Zephyr zone and global topics.
///
/// Decodes incoming gossip messages and dispatches them to the appropriate
/// handler. Invalid messages (decode failures, invalid proofs) are dropped
/// and never re-gossiped.
pub struct ZephyrGossipHandler {
    zone_topics: Arc<RwLock<HashSet<String>>>,
    global_topic: String,
    zone_message_tx: tokio::sync::mpsc::Sender<(String, ZephyrZoneMessage)>,
    global_message_tx: tokio::sync::mpsc::Sender<ZephyrGlobalMessage>,
}

impl ZephyrGossipHandler {
    pub fn new(
        global_topic: String,
        zone_message_tx: tokio::sync::mpsc::Sender<(String, ZephyrZoneMessage)>,
        global_message_tx: tokio::sync::mpsc::Sender<ZephyrGlobalMessage>,
    ) -> Self {
        Self {
            zone_topics: Arc::new(RwLock::new(HashSet::new())),
            global_topic,
            zone_message_tx,
            global_message_tx,
        }
    }

    /// Register a zone topic for handling.
    pub fn add_zone_topic(&self, topic: String) {
        if let Ok(mut topics) = self.zone_topics.write() {
            topics.insert(topic);
        }
    }

    /// Unregister a zone topic.
    pub fn remove_zone_topic(&self, topic: &str) {
        if let Ok(mut topics) = self.zone_topics.write() {
            topics.remove(topic);
        }
    }

    fn is_zone_topic(&self, topic: &str) -> bool {
        self.zone_topics
            .read()
            .map(|t| t.contains(topic))
            .unwrap_or(false)
    }
}

#[async_trait]
impl ServiceGossipHandler for ZephyrGossipHandler {
    fn handles_topic(&self, topic: &str) -> bool {
        topic == self.global_topic || self.is_zone_topic(topic)
    }

    async fn on_gossip(&self, topic: &str, data: &[u8], sender: Option<String>) {
        let sender_label = sender.as_deref().unwrap_or("unknown");

        if topic == self.global_topic {
            match grid_core::decode_canonical::<ZephyrGlobalMessage>(data) {
                Ok(msg) => {
                    debug!(%topic, %sender_label, "received global message");
                    if self.global_message_tx.send(msg).await.is_err() {
                        warn!("global message channel closed");
                    }
                }
                Err(e) => {
                    warn!(%topic, %sender_label, error = %e, "failed to decode global gossip");
                }
            }
        } else if self.is_zone_topic(topic) {
            match grid_core::decode_canonical::<ZephyrZoneMessage>(data) {
                Ok(msg) => {
                    debug!(%topic, %sender_label, "received zone message");
                    if self
                        .zone_message_tx
                        .send((topic.to_owned(), msg))
                        .await
                        .is_err()
                    {
                        warn!("zone message channel closed");
                    }
                }
                Err(e) => {
                    warn!(%topic, %sender_label, error = %e, "failed to decode zone gossip");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_programs_zephyr::{
        EpochAnnouncement, Nullifier, NoteCommitment, RejectReason, SpendReject,
        SpendTransaction,
    };

    fn make_handler() -> (
        ZephyrGossipHandler,
        tokio::sync::mpsc::Receiver<(String, ZephyrZoneMessage)>,
        tokio::sync::mpsc::Receiver<ZephyrGlobalMessage>,
    ) {
        let (zone_tx, zone_rx) = tokio::sync::mpsc::channel(32);
        let (global_tx, global_rx) = tokio::sync::mpsc::channel(32);
        let handler = ZephyrGossipHandler::new(
            "prog/global_topic_hex".to_owned(),
            zone_tx,
            global_tx,
        );
        (handler, zone_rx, global_rx)
    }

    #[test]
    fn handles_registered_topics() {
        let (handler, _, _) = make_handler();
        assert!(handler.handles_topic("prog/global_topic_hex"));
        assert!(!handler.handles_topic("prog/unknown"));

        handler.add_zone_topic("prog/zone_0".to_owned());
        assert!(handler.handles_topic("prog/zone_0"));

        handler.remove_zone_topic("prog/zone_0");
        assert!(!handler.handles_topic("prog/zone_0"));
    }

    #[tokio::test]
    async fn dispatches_global_message() {
        let (handler, _, mut global_rx) = make_handler();
        let msg = ZephyrGlobalMessage::EpochAnnounce(EpochAnnouncement {
            epoch: 1,
            randomness_seed: [0; 32],
            start_time_ms: 1000,
        });
        let data = grid_core::encode_canonical(&msg).unwrap();

        handler
            .on_gossip("prog/global_topic_hex", &data, Some("peer1".into()))
            .await;

        let received = global_rx.try_recv().unwrap();
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn dispatches_zone_message() {
        let (handler, mut zone_rx, _) = make_handler();
        handler.add_zone_topic("prog/zone_5".to_owned());

        let msg = ZephyrZoneMessage::Reject(SpendReject {
            nullifier: Nullifier([1; 32]),
            reason: RejectReason::DuplicateNullifier,
        });
        let data = grid_core::encode_canonical(&msg).unwrap();

        handler
            .on_gossip("prog/zone_5", &data, None)
            .await;

        let (topic, received) = zone_rx.try_recv().unwrap();
        assert_eq!(topic, "prog/zone_5");
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn invalid_data_dropped() {
        let (handler, mut zone_rx, _) = make_handler();
        handler.add_zone_topic("prog/zone_0".to_owned());

        handler
            .on_gossip("prog/zone_0", &[0xFF, 0xFF], None)
            .await;

        assert!(zone_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn submit_spend_routed_to_zone() {
        let (handler, mut zone_rx, _) = make_handler();
        handler.add_zone_topic("prog/zone_1".to_owned());

        let msg = ZephyrZoneMessage::SubmitSpend(SpendTransaction {
            input_commitment: NoteCommitment([0; 32]),
            nullifier: Nullifier([1; 32]),
            outputs: vec![],
            proof: vec![2, 3],
            public_signals: vec![],
        });
        let data = grid_core::encode_canonical(&msg).unwrap();

        handler
            .on_gossip("prog/zone_1", &data, Some("peer2".into()))
            .await;

        let (_, received) = zone_rx.try_recv().unwrap();
        assert_eq!(received, msg);
    }
}
