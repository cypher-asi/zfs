use std::collections::HashMap;

use grid_programs_zephyr::{Block, Nullifier, ZephyrConsensusMessage, ZephyrGlobalMessage};
use tracing::{debug, info, warn};

use crate::consensus::ConsensusAction;
use crate::publishing::{
    apply_certificate_locally, cache_block_txs, cleanup_mempool_after_cert, publish_action,
};
use crate::storage::NullifierSet;
use crate::zone_task::ZoneTaskState;

/// Persist finalized block nullifiers into the nullifier set.
///
/// Extracted as a free function to avoid borrow-checker conflicts when the
/// consensus engine holds a mutable reference into `ZoneTaskState.engine`.
fn persist_nullifiers_inline(
    block_hash: &[u8; 32],
    block_nullifiers: &HashMap<[u8; 32], (u32, Vec<Nullifier>)>,
    nullifier_set: &mut NullifierSet,
    zone_id: u32,
) {
    if let Some((_, nullifiers)) = block_nullifiers.get(block_hash) {
        for n in nullifiers {
            if let Err(e) = nullifier_set.insert(n.clone()) {
                warn!(zone_id, error = %e, "failed to persist nullifier");
            }
        }
    }
}

impl ZoneTaskState {
    pub(crate) fn handle_consensus_batch(&mut self, batch: Vec<ZephyrConsensusMessage>) {
        for cmsg in batch {
            match cmsg {
                ZephyrConsensusMessage::Proposal(proposal) => {
                    self.handle_proposal(proposal);
                }
                ZephyrConsensusMessage::Vote(vote) => {
                    self.handle_vote(vote);
                }
                ZephyrConsensusMessage::Reject(_) => {}
            }
        }
        self.resolve_deferred_cleanups();
    }

    fn handle_proposal(&mut self, proposal: Block) {
        info!(
            zone_id = self.zone_id,
            proposer = %hex::encode(&proposal.header.proposer_id[..8]),
            block_hash = %hex::encode(&proposal.block_hash[..8]),
            tx_count = proposal.transactions.len(),
            height = proposal.header.height,
            parent_hash = %hex::encode(&proposal.header.parent_hash[..8]),
            "received proposal from network"
        );

        if !self.verify_proposal_transactions(&proposal) {
            return;
        }

        cache_block_txs(
            &mut self.block_tx_cache,
            &mut self.block_nullifiers,
            self.zone_id,
            &proposal,
        );
        let Some(ref mut eng) = self.engine else { return };
        let identity = &self.identity;
        if let Some(action) = eng.vote_on_proposal(&proposal, |data| identity.sign(data)) {
            eng.reset_timeout();
            let _ = eng.take_fork_recovery_used();
            self.publish_and_self_certify(action);
        } else if proposal.header.parent_hash != *eng.parent_hash()
            && proposal.header.epoch == eng.epoch()
            && proposal.header.height >= eng.height()
        {
            debug!(
                zone_id = self.zone_id,
                proposal_parent = %hex::encode(&proposal.header.parent_hash[..8]),
                local_parent = %eng.parent_hash_hex(),
                height = proposal.header.height,
                "buffering proposal for retry after cert"
            );
            self.last_buffered_proposal = Some(proposal);
        }
    }

    fn handle_vote(&mut self, vote: grid_programs_zephyr::BlockVote) {
        let Some(ref mut eng) = self.engine else { return };
        if let Some(action) = eng.receive_vote(vote) {
            if let ConsensusAction::BroadcastCertificate(ref cert) = action {
                persist_nullifiers_inline(
                    &cert.block_hash,
                    &self.block_nullifiers,
                    &mut self.nullifier_set,
                    self.zone_id,
                );
                apply_certificate_locally(
                    cert,
                    &self.zone_head_store,
                    &mut self.block_tx_cache,
                    &self.runtime,
                );
                cleanup_mempool_after_cert(
                    cert,
                    &self.mempool,
                    &mut self.block_nullifiers,
                    &mut self.deferred_cleanups,
                );
            }
            publish_action(
                &action,
                &self.consensus_topic,
                &self.global_topic,
                &self.publish_tx,
                &self.block_tx_cache,
                &self.block_nullifiers,
            );
        }
    }

    pub(crate) fn handle_global_batch(&mut self, batch: Vec<ZephyrGlobalMessage>) {
        for gmsg in batch {
            match gmsg {
                ZephyrGlobalMessage::Certificate { cert, tx_nullifiers, nullifiers } => {
                    self.handle_certificate(cert, tx_nullifiers, nullifiers);
                }
                ZephyrGlobalMessage::EpochAnnounce(ann) => {
                    debug!(zone_id = self.zone_id, epoch = ann.epoch, "received epoch announcement");
                }
            }
        }
    }

    fn handle_certificate(
        &mut self,
        cert: grid_programs_zephyr::FinalityCertificate,
        tx_nullifiers: Vec<String>,
        nullifiers: Vec<Nullifier>,
    ) {
        if !tx_nullifiers.is_empty() {
            self.block_tx_cache
                .entry(cert.block_hash)
                .or_insert_with(|| (cert.zone_id, tx_nullifiers));
        }

        if !nullifiers.is_empty()
            && !self.block_nullifiers.contains_key(&cert.block_hash)
        {
            self.block_nullifiers
                .insert(cert.block_hash, (cert.zone_id, nullifiers));
        }

        if let Some(ref mut eng) = self.engine {
            if eng.apply_certificate(&cert) {
                let _ = eng.take_fork_recovery_used();
                persist_nullifiers_inline(
                    &cert.block_hash,
                    &self.block_nullifiers,
                    &mut self.nullifier_set,
                    self.zone_id,
                );
                apply_certificate_locally(
                    &cert,
                    &self.zone_head_store,
                    &mut self.block_tx_cache,
                    &self.runtime,
                );
                cleanup_mempool_after_cert(
                    &cert,
                    &self.mempool,
                    &mut self.block_nullifiers,
                    &mut self.deferred_cleanups,
                );

                let applied = self.pending_certs.drain_applicable(eng);
                for pc in &applied {
                    persist_nullifiers_inline(
                        &pc.block_hash,
                        &self.block_nullifiers,
                        &mut self.nullifier_set,
                        self.zone_id,
                    );
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
                }
                self.pending_certs.purge_overflow();
                self.retry_buffered_proposal();
            } else if cert.block_hash == *eng.parent_hash() {
                debug!(
                    zone_id = self.zone_id,
                    cert_block = %hex::encode(&cert.block_hash[..8]),
                    "ignoring stale certificate (already applied)"
                );
            } else if !self.pending_certs.push(cert.clone()) {
                debug!(
                    zone_id = self.zone_id,
                    cert_block = %hex::encode(&cert.block_hash[..8]),
                    "pending cert buffer full or duplicate, dropping"
                );
            } else {
                debug!(
                    zone_id = self.zone_id,
                    cert_block = %hex::encode(&cert.block_hash[..8]),
                    cert_parent = %hex::encode(&cert.parent_hash[..8]),
                    local_parent = %eng.parent_hash_hex(),
                    buffered = self.pending_certs.len(),
                    "buffering out-of-order certificate"
                );
            }
        } else {
            persist_nullifiers_inline(
                &cert.block_hash,
                &self.block_nullifiers,
                &mut self.nullifier_set,
                self.zone_id,
            );
            apply_certificate_locally(
                &cert,
                &self.zone_head_store,
                &mut self.block_tx_cache,
                &self.runtime,
            );
            cleanup_mempool_after_cert(
                &cert,
                &self.mempool,
                &mut self.block_nullifiers,
                &mut self.deferred_cleanups,
            );
        }
    }

    pub(crate) fn retry_buffered_proposal(&mut self) {
        let proposal = match self.last_buffered_proposal.take() {
            Some(p) => p,
            None => return,
        };

        if !self.verify_proposal_transactions(&proposal) {
            return;
        }

        let Some(ref mut eng) = self.engine else { return };

        if proposal.header.height < eng.height() {
            debug!(
                zone_id = self.zone_id,
                proposal_height = proposal.header.height,
                local_height = eng.height(),
                "discarding stale buffered proposal"
            );
            return;
        }
        if proposal.header.parent_hash != *eng.parent_hash() {
            self.last_buffered_proposal = Some(proposal);
            return;
        }
        debug!(
            zone_id = self.zone_id,
            block_hash = %hex::encode(&proposal.block_hash[..8]),
            height = proposal.header.height,
            "retrying buffered proposal after cert catch-up"
        );
        let identity = &self.identity;
        if let Some(action) = eng.vote_on_proposal(&proposal, |data| identity.sign(data)) {
            eng.reset_timeout();
            let _ = eng.take_fork_recovery_used();
            self.publish_and_self_certify(action);
        }
    }

    fn resolve_deferred_cleanups(&mut self) {
        let resolved: Vec<[u8; 32]> = self
            .deferred_cleanups
            .keys()
            .filter(|h| self.block_nullifiers.contains_key(*h))
            .copied()
            .collect();
        for hash in resolved {
            if self.deferred_cleanups.remove(&hash).is_some() {
                if let Some((_, nullifiers)) = self.block_nullifiers.remove(&hash) {
                    self.mempool.remove_nullifiers(self.zone_id, &nullifiers);
                }
            }
        }
    }

    /// Verify spend proofs and nullifier freshness for all transactions in a
    /// proposal.  Returns `false` (reject) if any check fails.
    fn verify_proposal_transactions(&self, proposal: &Block) -> bool {
        if let Some(ref verifier) = self.proof_verifier {
            for (i, tx) in proposal.transactions.iter().enumerate() {
                let signals: Result<Vec<ark_bn254::Fr>, _> = tx
                    .public_signals
                    .iter()
                    .map(|b| {
                        use ark_serialize::CanonicalDeserialize;
                        ark_bn254::Fr::deserialize_compressed(&b[..])
                            .map_err(|_| ())
                    })
                    .collect();
                let Ok(signals) = signals else {
                    warn!(
                        zone_id = self.zone_id,
                        block_hash = %hex::encode(&proposal.block_hash[..8]),
                        tx_index = i,
                        "rejecting proposal: invalid public signal encoding"
                    );
                    return false;
                };
                if verifier.verify(&tx.proof, &signals).is_err() {
                    warn!(
                        zone_id = self.zone_id,
                        block_hash = %hex::encode(&proposal.block_hash[..8]),
                        tx_index = i,
                        "rejecting proposal: spend proof verification failed"
                    );
                    return false;
                }
            }
        }

        for (i, tx) in proposal.transactions.iter().enumerate() {
            if self.nullifier_set.contains(&tx.nullifier) {
                warn!(
                    zone_id = self.zone_id,
                    block_hash = %hex::encode(&proposal.block_hash[..8]),
                    tx_index = i,
                    nullifier = %hex::encode(&tx.nullifier.0[..8]),
                    "rejecting proposal: nullifier already spent"
                );
                return false;
            }
        }

        true
    }

    /// Insert nullifiers from a finalized block into the persistent set.
    pub(crate) fn persist_finalized_nullifiers(&mut self, block_hash: &[u8; 32]) {
        if let Some((_, nullifiers)) = self.block_nullifiers.get(block_hash) {
            for n in nullifiers {
                if let Err(e) = self.nullifier_set.insert(n.clone()) {
                    warn!(
                        zone_id = self.zone_id,
                        error = %e,
                        "failed to persist nullifier"
                    );
                }
            }
        }
    }

    /// Publish an action, then self-certify if it's a vote (leader self-vote path).
    pub(crate) fn publish_and_self_certify(&mut self, action: ConsensusAction) {
        publish_action(
            &action,
            &self.consensus_topic,
            &self.global_topic,
            &self.publish_tx,
            &self.block_tx_cache,
            &self.block_nullifiers,
        );
        if let ConsensusAction::BroadcastVote(vote) = action {
            let Some(ref mut eng) = self.engine else { return };
            if let Some(cert_action) = eng.receive_vote(vote) {
                if let ConsensusAction::BroadcastCertificate(ref cert) = cert_action {
                    persist_nullifiers_inline(
                        &cert.block_hash,
                        &self.block_nullifiers,
                        &mut self.nullifier_set,
                        self.zone_id,
                    );
                    apply_certificate_locally(
                        cert,
                        &self.zone_head_store,
                        &mut self.block_tx_cache,
                        &self.runtime,
                    );
                    cleanup_mempool_after_cert(
                        cert,
                        &self.mempool,
                        &mut self.block_nullifiers,
                        &mut self.deferred_cleanups,
                    );
                }
                publish_action(
                    &cert_action,
                    &self.consensus_topic,
                    &self.global_topic,
                    &self.publish_tx,
                    &self.block_tx_cache,
                    &self.block_nullifiers,
                );
            }
        }
    }
}
