use std::time::Duration;

use grid_service::TopicCommand;
use tracing::{debug, info, warn};

use crate::committee::sample_committee;
use crate::consensus::ConsensusAction;
use crate::publishing::{
    apply_certificate_locally, cache_block_txs, cleanup_mempool_after_cert, publish_action,
};
use crate::consensus::ZoneConsensus;
use crate::zone_task::ZoneTaskState;

impl ZoneTaskState {
    pub(crate) async fn handle_tick(&mut self, elapsed: Duration) {
        let epoch_elapsed = elapsed.as_millis() as u64 % self.config.epoch_duration_ms;
        let progress = epoch_elapsed as f32 / self.config.epoch_duration_ms as f32;
        let expected_epoch = elapsed.as_millis() as u64 / self.config.epoch_duration_ms;

        if expected_epoch > self.last_known_epoch {
            self.check_epoch_transition(expected_epoch).await;
        }

        {
            let mut rt = self.runtime.write();
            rt.epoch_progress_pct = progress;
            rt.current_epoch = self.last_known_epoch;
        }

        if let Some(mut eng) = self.engine.take() {
            eng.tick();
            if eng.is_round_timed_out(self.config.round_timeout_ticks) {
                self.handle_timeout(&mut eng);
            }
            self.periodic_drain(&mut eng);
            self.periodic_health(&eng);
            self.try_propose(&mut eng);
            self.engine = Some(eng);
        }

        let zone_len = self.mempool.len(self.zone_id);
        {
            let mut rt = self.runtime.write();
            rt.mempool_sizes.insert(self.zone_id, zone_len);
        }
    }

    async fn check_epoch_transition(&mut self, expected_epoch: u64) {
        let mut em = self.epoch_mgr.lock().await;
        while em.current_epoch() < expected_epoch {
            em.advance_epoch(&self.my_validator_id);
            info!(
                zone_id = self.zone_id,
                new_epoch = em.current_epoch(),
                "epoch advanced"
            );
        }

        let current_epoch = em.current_epoch();
        if current_epoch <= self.last_known_epoch {
            return;
        }
        self.last_known_epoch = current_epoch;
        let assigned = em.zones_for_validator(&self.my_validator_id);
        let is_assigned = assigned.contains(&self.zone_id);
        let was_assigned = self.engine.is_some();

        {
            let mut rt = self.runtime.write();
            rt.assigned_zones = assigned;
        }

        if is_assigned {
            let committee = sample_committee(
                em.randomness_seed(),
                self.zone_id,
                &self.validators,
                self.config.committee_size,
            );
            drop(em);
            if !was_assigned {
                let _ = self.topic_tx.try_send(TopicCommand::Subscribe(self.zone_topic.clone()));
                let _ = self.topic_tx.try_send(TopicCommand::Subscribe(self.consensus_topic.clone()));
                info!(zone_id = self.zone_id, "epoch transition: gained zone, subscribed to topics");
            }
            if let Some(ref mut eng) = self.engine {
                if eng.consecutive_timeouts() >= 2 {
                    warn!(
                        zone_id = self.zone_id,
                        consecutive_timeouts = eng.consecutive_timeouts(),
                        height = eng.height(),
                        parent_hash = %eng.parent_hash_hex(),
                        "zone stalled at epoch boundary, enabling fork recovery"
                    );
                    eng.enable_fork_recovery();
                }
                eng.advance_to_epoch(current_epoch, committee);
                self.pending_certs.retain_epoch(current_epoch);
            } else {
                let prev_head = self
                    .zone_head_store
                    .get(&self.zone_id)
                    .map(|v| *v)
                    .unwrap_or([0u8; 32]);
                self.engine = Some(ZoneConsensus::new(
                    self.zone_id,
                    current_epoch,
                    committee,
                    self.my_validator_id,
                    prev_head,
                    self.config.clone(),
                    self.zone_id as usize,
                ));
                self.mempool.add_zone(self.zone_id, 65_536);
                self.pending_certs.retain_epoch(current_epoch);
            }
        } else {
            drop(em);
            if was_assigned {
                let _ = self.topic_tx.try_send(TopicCommand::Unsubscribe(self.zone_topic.clone()));
                let _ = self.topic_tx.try_send(TopicCommand::Unsubscribe(self.consensus_topic.clone()));
                info!(zone_id = self.zone_id, "epoch transition: lost zone, unsubscribed from topics");
                self.engine = None;
                self.mempool.remove_zone(self.zone_id);
                self.pending_certs.clear();
            }
        }
    }

    fn handle_timeout(&mut self, eng: &mut ZoneConsensus) {
        let effective_timeout =
            self.config.round_timeout_ticks * (1 + eng.consecutive_timeouts().min(3));
        let (vote_blocks, max_votes) = eng.vote_summary();
        warn!(
            zone_id = self.zone_id,
            round = eng.round(),
            height = eng.height(),
            consecutive_timeouts = eng.consecutive_timeouts(),
            effective_timeout_ticks = effective_timeout,
            is_leader = eng.is_leader(),
            leader = %hex::encode(&eng.leader_id()[..8]),
            parent_hash = %eng.parent_hash_hex(),
            proposal_seen = eng.proposal_seen(),
            has_pending_proposal = eng.has_pending_proposal(),
            votes_for_pending = eng.vote_count_for_pending(),
            vote_blocks,
            max_votes,
            committee_size = eng.committee_size(),
            quorum = self.config.quorum_threshold,
            pending_certs = self.pending_certs.len(),
            "round timed out without quorum, rotating leader"
        );
        let abandoned_txs = eng.timeout_round();
        if !abandoned_txs.is_empty() {
            self.mempool.reinsert_batch(self.zone_id, abandoned_txs);
        }
        if eng.consecutive_timeouts() >= 2 || self.pending_certs.len() >= 8 {
            if eng.enable_fork_recovery() {
                warn!(
                    zone_id = self.zone_id,
                    consecutive_timeouts = eng.consecutive_timeouts(),
                    pending_certs = self.pending_certs.len(),
                    height = eng.height(),
                    parent_hash = %eng.parent_hash_hex(),
                    mempool = self.mempool.len(self.zone_id),
                    "zone stalled, enabling fork recovery"
                );
            }
        }
        {
            let mut rt = self.runtime.write();
            rt.zone_consecutive_timeouts
                .insert(self.zone_id, eng.consecutive_timeouts());
        }
    }

    fn periodic_drain(&mut self, eng: &mut ZoneConsensus) {
        if eng.ticks_in_round() % 10 != 0 || self.pending_certs.is_empty() {
            return;
        }
        let applied = self.pending_certs.drain_applicable(eng);
        for pc in &applied {
            apply_certificate_locally(
                pc,
                &self.zone_head_store,
                &mut self.block_tx_cache,
                &self.runtime,
            );
            cleanup_mempool_after_cert(
                pc,
                &self.mempool,
                &mut self.block_nullifiers,
                &mut self.deferred_cleanups,
            );
            debug!(zone_id = self.zone_id, "periodic drain: applied buffered certificate");
        }
        if !applied.is_empty() {
            self.retry_buffered_proposal();
        }
    }

    fn periodic_health(&self, eng: &ZoneConsensus) {
        if eng.consecutive_timeouts() == 0 || eng.ticks_in_round() % 100 != 0 {
            return;
        }
        let (vote_blocks, max_votes) = eng.vote_summary();
        let distinct_parents: std::collections::HashSet<[u8; 32]> =
            self.pending_certs.iter().map(|c| c.parent_hash).collect();
        info!(
            zone_id = self.zone_id,
            round = eng.round(),
            height = eng.height(),
            epoch = eng.epoch(),
            consecutive_timeouts = eng.consecutive_timeouts(),
            consecutive_successes = eng.consecutive_successes(),
            ticks_in_round = eng.ticks_in_round(),
            is_leader = eng.is_leader(),
            leader = %hex::encode(&eng.leader_id()[..8]),
            proposal_seen = eng.proposal_seen(),
            has_pending_proposal = eng.has_pending_proposal(),
            votes_for_pending = eng.vote_count_for_pending(),
            vote_blocks,
            max_votes,
            parent_hash = %eng.parent_hash_hex(),
            pending_certs = self.pending_certs.len(),
            pending_cert_distinct_parents = distinct_parents.len(),
            mempool_len = self.mempool.len(self.zone_id),
            "zone stall health check"
        );
    }

    fn try_propose(&mut self, eng: &mut ZoneConsensus) {
        if !eng.is_leader() || eng.in_warmup() {
            return;
        }
        let is_rebroadcast = eng.has_pending_proposal();
        let spends = if is_rebroadcast {
            vec![]
        } else {
            self.mempool
                .drain_proposal(self.zone_id, self.config.max_block_size)
        };
        let tx_count = spends.len();
        let identity = &self.identity;
        let Some(action) = eng.propose(spends, |data| identity.sign(data)) else {
            return;
        };
        let ConsensusAction::BroadcastProposal(ref block) = action else {
            return;
        };
        if is_rebroadcast {
            debug!(
                zone_id = self.zone_id,
                round = eng.round(),
                block_hash = %hex::encode(&block.block_hash[..8]),
                "rebroadcasting proposal"
            );
        } else {
            info!(
                zone_id = self.zone_id,
                height = eng.height(),
                round = eng.round(),
                tx_count,
                block_hash = %hex::encode(&block.block_hash[..8]),
                "proposed new block"
            );
            cache_block_txs(
                &mut self.block_tx_cache,
                &mut self.block_nullifiers,
                self.zone_id,
                block,
            );
        }
        publish_action(
            &action,
            &self.consensus_topic,
            &self.global_topic,
            &self.publish_tx,
            &self.block_tx_cache,
        );
        if !is_rebroadcast {
            let identity2 = &self.identity;
            if let Some(vote_action) =
                eng.vote_on_proposal(block, |data| identity2.sign(data))
            {
                self.publish_and_self_certify(vote_action);
            }
        }
    }
}
