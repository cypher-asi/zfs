use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use grid_programs_zephyr::{
    Block, Nullifier, ValidatorInfo, ZephyrConsensusMessage, ZephyrGlobalMessage,
};
use grid_service::TopicCommand;
use tokio::sync::mpsc;
use tracing::debug;

use crate::config::ZephyrConfig;
use crate::consensus::pending_certs::PendingCertBuffer;
use crate::consensus::ZoneConsensus;
use crate::epoch::EpochManager;
use crate::service::ZephyrRuntime;
use crate::shared_mempool::SharedMempool;

pub(crate) struct ZoneTaskState {
    pub zone_id: u32,
    pub engine: Option<ZoneConsensus>,
    pub pending_certs: PendingCertBuffer,
    pub block_tx_cache: HashMap<[u8; 32], (u32, Vec<String>)>,
    pub block_nullifiers: HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    pub deferred_cleanups: HashMap<[u8; 32], u32>,
    pub last_buffered_proposal: Option<Block>,
    pub last_known_epoch: u64,
    pub validators: Vec<ValidatorInfo>,
    pub my_validator_id: [u8; 32],
    pub config: ZephyrConfig,
    pub publish_tx: mpsc::Sender<(String, Vec<u8>)>,
    pub topic_tx: mpsc::Sender<TopicCommand>,
    pub zone_topic: String,
    pub consensus_topic: String,
    pub global_topic: String,
    pub zone_head_store: Arc<DashMap<u32, [u8; 32]>>,
    pub mempool: SharedMempool,
    pub runtime: Arc<parking_lot::RwLock<ZephyrRuntime>>,
    pub epoch_mgr: Arc<tokio::sync::Mutex<EpochManager>>,
}

impl ZoneTaskState {
    pub async fn run(
        mut self,
        mut proposal_rx: mpsc::Receiver<ZephyrConsensusMessage>,
        mut vote_rx: mpsc::Receiver<ZephyrConsensusMessage>,
        mut global_rx: mpsc::Receiver<ZephyrGlobalMessage>,
        shutdown: tokio_util::sync::CancellationToken,
    ) {
        let round_interval =
            std::time::Duration::from_millis(self.config.round_interval_ms);
        let mut round_timer = tokio::time::interval(round_interval);
        round_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let epoch_start = tokio::time::Instant::now();

        loop {
            tokio::select! {
                biased;

                _ = shutdown.cancelled() => {
                    debug!(zone_id = self.zone_id, "zone consensus task shutting down");
                    break;
                }

                _ = round_timer.tick() => {
                    Self::drain_consensus_channel(&mut self, &mut proposal_rx, 512);
                    Self::drain_consensus_channel(&mut self, &mut vote_rx, 512);
                    let elapsed = epoch_start.elapsed();
                    self.handle_tick(elapsed).await;
                }

                msg = proposal_rx.recv() => {
                    let Some(first) = msg else { break };
                    let mut batch = Vec::with_capacity(65);
                    batch.push(first);
                    while batch.len() < 64 {
                        match proposal_rx.try_recv() {
                            Ok(m) => batch.push(m),
                            Err(_) => break,
                        }
                    }
                    self.handle_consensus_batch(batch);
                }

                msg = vote_rx.recv() => {
                    let Some(first) = msg else { break };
                    let mut batch = Vec::with_capacity(129);
                    batch.push(first);
                    while batch.len() < 128 {
                        match vote_rx.try_recv() {
                            Ok(m) => batch.push(m),
                            Err(_) => break,
                        }
                    }
                    self.handle_consensus_batch(batch);
                }

                msg = global_rx.recv() => {
                    let Some(first) = msg else { break };
                    let mut batch = Vec::with_capacity(33);
                    batch.push(first);
                    while batch.len() < 32 {
                        match global_rx.try_recv() {
                            Ok(m) => batch.push(m),
                            Err(_) => break,
                        }
                    }
                    self.handle_global_batch(batch);
                }
            }
        }
    }

    /// Drain all available messages from a consensus channel before processing
    /// a tick. This prevents premature timeouts when votes/proposals are
    /// pending in the channel but haven't been processed yet.
    fn drain_consensus_channel(
        this: &mut Self,
        rx: &mut mpsc::Receiver<ZephyrConsensusMessage>,
        limit: usize,
    ) {
        let mut batch = Vec::new();
        while batch.len() < limit {
            match rx.try_recv() {
                Ok(m) => batch.push(m),
                Err(_) => break,
            }
        }
        if !batch.is_empty() {
            this.handle_consensus_batch(batch);
        }
    }
}
