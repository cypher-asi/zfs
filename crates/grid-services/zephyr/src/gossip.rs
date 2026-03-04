use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use grid_programs_zephyr::{ZephyrConsensusMessage, ZephyrGlobalMessage, ZephyrZoneMessage};
use grid_service::ServiceGossipHandler;
use tracing::{debug, warn};

/// Gossip handler for Zephyr zone, consensus, and global topics.
///
/// Decodes incoming gossip messages and dispatches them to the appropriate
/// handler. Invalid messages (decode failures, invalid proofs) are dropped
/// and never re-gossiped.
pub struct ZephyrGossipHandler {
    zone_topics: Arc<RwLock<HashSet<String>>>,
    consensus_topics: Arc<RwLock<HashSet<String>>>,
    global_topic: String,
    /// Proposals on dedicated consensus topics -- small, high-priority channel.
    consensus_proposal_tx: tokio::sync::mpsc::Sender<(String, ZephyrConsensusMessage)>,
    /// Votes and rejects on consensus topics -- larger channel.
    consensus_vote_tx: tokio::sync::mpsc::Sender<(String, ZephyrConsensusMessage)>,
    /// Spend submissions only -- high-volume, loss-tolerant.
    zone_message_tx: tokio::sync::mpsc::Sender<(String, ZephyrZoneMessage)>,
    global_message_tx: tokio::sync::mpsc::Sender<ZephyrGlobalMessage>,
}

impl ZephyrGossipHandler {
    pub fn new(
        global_topic: String,
        consensus_proposal_tx: tokio::sync::mpsc::Sender<(String, ZephyrConsensusMessage)>,
        consensus_vote_tx: tokio::sync::mpsc::Sender<(String, ZephyrConsensusMessage)>,
        zone_message_tx: tokio::sync::mpsc::Sender<(String, ZephyrZoneMessage)>,
        global_message_tx: tokio::sync::mpsc::Sender<ZephyrGlobalMessage>,
    ) -> Self {
        Self {
            zone_topics: Arc::new(RwLock::new(HashSet::new())),
            consensus_topics: Arc::new(RwLock::new(HashSet::new())),
            global_topic,
            consensus_proposal_tx,
            consensus_vote_tx,
            zone_message_tx,
            global_message_tx,
        }
    }

    /// Register a zone spend topic for handling.
    pub fn add_zone_topic(&self, topic: String) {
        if let Ok(mut topics) = self.zone_topics.write() {
            topics.insert(topic);
        }
    }

    /// Register a zone consensus topic for handling.
    pub fn add_consensus_topic(&self, topic: String) {
        if let Ok(mut topics) = self.consensus_topics.write() {
            topics.insert(topic);
        }
    }

    /// Unregister a zone spend topic.
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

    fn is_consensus_topic(&self, topic: &str) -> bool {
        self.consensus_topics
            .read()
            .map(|t| t.contains(topic))
            .unwrap_or(false)
    }
}

#[async_trait]
impl ServiceGossipHandler for ZephyrGossipHandler {
    fn handles_topic(&self, topic: &str) -> bool {
        topic == self.global_topic || self.is_zone_topic(topic) || self.is_consensus_topic(topic)
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
        } else if self.is_consensus_topic(topic) {
            let decode_start = std::time::Instant::now();
            match grid_core::decode_canonical::<ZephyrConsensusMessage>(data) {
                Ok(msg) => {
                    let is_proposal = matches!(&msg, ZephyrConsensusMessage::Proposal(_));
                    if is_proposal {
                        // #region agent log
                        {
                            let decode_us = decode_start.elapsed().as_micros();
                            use std::io::Write;
                            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("debug-6fcc0e.log") {
                                let _ = writeln!(f, r#"{{"sessionId":"6fcc0e","hypothesisId":"N","location":"gossip.rs:proposal_decode","message":"proposal decoded","data":{{"bytes":{},"decode_us":{},"sender":"{}"}},"timestamp":{}}}"#,
                                    data.len(), decode_us, sender_label,
                                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
                            }
                        }
                        // #endregion
                        debug!(%topic, %sender_label, "received consensus proposal");
                        if self
                            .consensus_proposal_tx
                            .send((topic.to_owned(), msg))
                            .await
                            .is_err()
                        {
                            warn!("consensus proposal channel closed");
                        }
                    } else {
                        let msg_type = match &msg {
                            ZephyrConsensusMessage::Vote(_) => "Vote",
                            ZephyrConsensusMessage::Reject(_) => "Reject",
                            _ => unreachable!(),
                        };
                        debug!(%topic, %sender_label, msg_type, "received consensus message");
                        if let Err(e) =
                            self.consensus_vote_tx.try_send((topic.to_owned(), msg))
                        {
                            warn!(msg_type, "consensus vote channel full, dropping: {e}");
                        }
                    }
                }
                Err(e) => {
                    warn!(%topic, %sender_label, error = %e, "failed to decode consensus gossip");
                }
            }
        } else if self.is_zone_topic(topic) {
            match grid_core::decode_canonical::<ZephyrZoneMessage>(data) {
                Ok(msg) => {
                    let msg_type = match &msg {
                        ZephyrZoneMessage::SubmitSpend(_) => "SubmitSpend",
                        ZephyrZoneMessage::SubmitSpendBatch(_) => "SubmitSpendBatch",
                    };
                    debug!(%topic, %sender_label, msg_type, "received zone message");
                    if let Err(e) = self.zone_message_tx.try_send((topic.to_owned(), msg)) {
                        warn!("zone_tx full, dropping spend: {e}");
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
        EpochAnnouncement, NoteCommitment, Nullifier, RejectReason, SpendReject, SpendTransaction,
    };

    fn make_handler() -> (
        ZephyrGossipHandler,
        tokio::sync::mpsc::Receiver<(String, ZephyrConsensusMessage)>,
        tokio::sync::mpsc::Receiver<(String, ZephyrConsensusMessage)>,
        tokio::sync::mpsc::Receiver<(String, ZephyrZoneMessage)>,
        tokio::sync::mpsc::Receiver<ZephyrGlobalMessage>,
    ) {
        let (proposal_tx, proposal_rx) = tokio::sync::mpsc::channel(32);
        let (vote_tx, vote_rx) = tokio::sync::mpsc::channel(32);
        let (zone_tx, zone_rx) = tokio::sync::mpsc::channel(32);
        let (global_tx, global_rx) = tokio::sync::mpsc::channel(32);
        let handler = ZephyrGossipHandler::new(
            "prog/global_topic_hex".to_owned(),
            proposal_tx,
            vote_tx,
            zone_tx,
            global_tx,
        );
        (handler, proposal_rx, vote_rx, zone_rx, global_rx)
    }

    #[test]
    fn handles_registered_topics() {
        let (handler, _, _, _, _) = make_handler();
        assert!(handler.handles_topic("prog/global_topic_hex"));
        assert!(!handler.handles_topic("prog/unknown"));

        handler.add_zone_topic("prog/zone_0".to_owned());
        assert!(handler.handles_topic("prog/zone_0"));

        handler.add_consensus_topic("prog/cons_0".to_owned());
        assert!(handler.handles_topic("prog/cons_0"));

        handler.remove_zone_topic("prog/zone_0");
        assert!(!handler.handles_topic("prog/zone_0"));
        assert!(handler.handles_topic("prog/cons_0"));
    }

    #[tokio::test]
    async fn dispatches_global_message() {
        let (handler, _, _, _, mut global_rx) = make_handler();
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
    async fn dispatches_consensus_vote_message() {
        let (handler, _, mut vote_rx, _, _) = make_handler();
        handler.add_consensus_topic("prog/cons_5".to_owned());

        let msg = ZephyrConsensusMessage::Reject(SpendReject {
            nullifier: Nullifier([1; 32]),
            reason: RejectReason::DuplicateNullifier,
        });
        let data = grid_core::encode_canonical(&msg).unwrap();

        handler.on_gossip("prog/cons_5", &data, None).await;

        let (topic, received) = vote_rx.try_recv().unwrap();
        assert_eq!(topic, "prog/cons_5");
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn invalid_data_dropped() {
        let (handler, mut proposal_rx, mut vote_rx, mut zone_rx, _) = make_handler();
        handler.add_zone_topic("prog/zone_0".to_owned());
        handler.add_consensus_topic("prog/cons_0".to_owned());

        handler.on_gossip("prog/zone_0", &[0xFF, 0xFF], None).await;
        handler.on_gossip("prog/cons_0", &[0xFF, 0xFF], None).await;

        assert!(proposal_rx.try_recv().is_err());
        assert!(vote_rx.try_recv().is_err());
        assert!(zone_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn submit_spend_routed_to_zone() {
        let (handler, _, _, mut zone_rx, _) = make_handler();
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
