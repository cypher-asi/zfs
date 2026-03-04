use grid_programs_zephyr::{
    Block, BlockVote, EpochId, FinalityCertificate, SpendTransaction, ValidatorInfo, ZoneId,
};
use tracing::{debug, info, warn};

use super::block::{build_block, BlockParams};
use super::leader::leader_for_round;
use super::vote::CertificateBuilder;
use crate::config::ZephyrConfig;

/// Per-zone consensus state machine.
///
/// Each zone the validator is assigned to gets an independent `ZoneConsensus`
/// instance. It tracks the current round, collects votes, and coordinates
/// proposal/certification. `height` is a monotonic per-zone counter that
/// is not reset across epochs.
const MAX_PROPOSAL_REBROADCASTS: u32 = 5;
const STALL_DECAY_SUCCESSES: u32 = 2;
const WARMUP_TICKS: u32 = 30;

pub struct ZoneConsensus {
    zone_id: ZoneId,
    epoch: EpochId,
    round: u64,
    height: u64,
    committee: Vec<ValidatorInfo>,
    my_validator_id: [u8; 32],
    cert_builder: CertificateBuilder,
    parent_hash: [u8; 32],
    config: ZephyrConfig,
    pending_proposal: Option<Block>,
    rebroadcast_count: u32,
    ticks_in_round: u32,
    consecutive_timeouts: u32,
    consecutive_successes: u32,
    proposal_seen: bool,
    voted_block_hash: Option<[u8; 32]>,
    warmup_ticks: u32,
    force_adopt_next_cert: bool,
    fork_recovery_used: bool,
    node_id: usize,
}

/// Actions the consensus engine requests the caller to perform.
#[derive(Debug)]
pub enum ConsensusAction {
    BroadcastProposal(Block),
    BroadcastVote(BlockVote),
    BroadcastCertificate(FinalityCertificate),
}

impl ZoneConsensus {
    pub fn new(
        zone_id: ZoneId,
        epoch: EpochId,
        committee: Vec<ValidatorInfo>,
        my_validator_id: [u8; 32],
        parent_hash: [u8; 32],
        config: ZephyrConfig,
        node_id: usize,
    ) -> Self {
        let force_adopt_at_genesis = parent_hash == [0u8; 32];
        Self {
            zone_id,
            epoch,
            round: 0,
            height: 0,
            committee: committee.clone(),
            my_validator_id,
            cert_builder: CertificateBuilder::new(zone_id, epoch, config.quorum_threshold),
            parent_hash,
            config,
            pending_proposal: None,
            rebroadcast_count: 0,
            ticks_in_round: 0,
            consecutive_timeouts: 0,
            consecutive_successes: 0,
            proposal_seen: false,
            voted_block_hash: None,
            warmup_ticks: WARMUP_TICKS,
            force_adopt_next_cert: force_adopt_at_genesis,
            fork_recovery_used: false,
            node_id,
        }
    }

    pub fn zone_id(&self) -> ZoneId {
        self.zone_id
    }

    pub fn epoch(&self) -> EpochId {
        self.epoch
    }

    pub fn round(&self) -> u64 {
        self.round
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn node_id(&self) -> usize {
        self.node_id
    }

    pub fn is_leader(&self) -> bool {
        let leader = leader_for_round(&self.committee, self.epoch, self.round);
        leader.validator_id == self.my_validator_id
    }

    /// Increment the per-round tick counter.  Called once per round-timer fire.
    pub fn tick(&mut self) {
        self.ticks_in_round += 1;
        if self.warmup_ticks > 0 {
            self.warmup_ticks -= 1;
        }
    }

    /// Whether the node is still in the initial warmup period.
    pub fn in_warmup(&self) -> bool {
        self.warmup_ticks > 0
    }

    /// Whether the current round has exceeded the timeout threshold.
    pub fn is_round_timed_out(&self, timeout_ticks: u32) -> bool {
        let effective_timeout = timeout_ticks * (1 + self.consecutive_timeouts.min(3));
        self.ticks_in_round >= effective_timeout
    }

    /// Reset the round timeout counter (called when consensus activity is
    /// observed, e.g. receiving a valid proposal).
    pub fn reset_timeout(&mut self) {
        self.ticks_in_round = 0;
    }

    /// Advance to the next round without a finalized block.  Rotates the
    /// leader while preserving `parent_hash` and `height` (no block was
    /// committed).  Returns the transactions from the abandoned proposal so the
    /// caller can re-insert them into the mempool.
    pub fn timeout_round(&mut self) -> Vec<SpendTransaction> {
        let txs = self
            .pending_proposal
            .take()
            .map(|b| b.transactions)
            .unwrap_or_default();
        self.round += 1;
        self.rebroadcast_count = 0;
        self.ticks_in_round = 0;
        self.consecutive_timeouts += 1;
        self.consecutive_successes = 0;
        self.proposal_seen = false;
        self.voted_block_hash = None;
        self.cert_builder.clear_votes();
        txs
    }

    /// Called by the leader when the round timer fires.
    ///
    /// On the first call in a round, builds a new block from `spends` and
    /// caches it.  Re-broadcasts the cached block up to
    /// `MAX_PROPOSAL_REBROADCASTS` times, then returns `None` to avoid
    /// flooding GossipSub.  The caller should only drain the mempool when
    /// `has_pending_proposal()` is false.
    pub fn propose(
        &mut self,
        spends: Vec<SpendTransaction>,
        sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Option<ConsensusAction> {
        if !self.is_leader() {
            return None;
        }

        if let Some(ref block) = self.pending_proposal {
            if self.rebroadcast_count >= MAX_PROPOSAL_REBROADCASTS {
                debug!(
                    zone_id = self.zone_id,
                    node_id = self.node_id,
                    round = self.round,
                    block_hash = %hex::encode(&block.block_hash[..8]),
                    votes = self.cert_builder.vote_count(&block.block_hash),
                    quorum = self.config.quorum_threshold,
                    "proposal rebroadcast limit reached, waiting for quorum or timeout"
                );
                return None;
            }
            self.rebroadcast_count += 1;
            return Some(ConsensusAction::BroadcastProposal(block.clone()));
        }

        let max = self.config.max_block_size.min(spends.len());
        let block_spends = spends.into_iter().take(max).collect();

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let params = BlockParams {
            zone_id: self.zone_id,
            epoch: self.epoch,
            height: self.height,
            parent_hash: self.parent_hash,
            timestamp_ms: now_ms,
            proposer_id: self.my_validator_id,
        };

        let block = build_block(params, block_spends, sign_fn);
        self.pending_proposal = Some(block.clone());

        Some(ConsensusAction::BroadcastProposal(block))
    }

    /// Whether a proposal is already pending for this round.
    /// When true, the caller should skip draining the mempool since those
    /// transactions are already in the cached block.
    pub fn has_pending_proposal(&self) -> bool {
        self.pending_proposal.is_some()
    }

    /// Check all guard conditions on a proposal. Returns `Ok(())` if the
    /// proposal is valid for voting, or `Err(reason)` explaining rejection.
    fn validate_proposal(&self, proposal: &Block) -> Result<(), &'static str> {
        if self.proposal_seen {
            debug!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                block_hash = %hex::encode(&proposal.block_hash[..8]),
                "ignoring proposal: already voted this round"
            );
            return Err("already voted");
        }
        if proposal.header.zone_id != self.zone_id || proposal.header.epoch != self.epoch {
            warn!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                proposal_zone = proposal.header.zone_id,
                proposal_epoch = proposal.header.epoch,
                local_epoch = self.epoch,
                "rejecting proposal: zone/epoch mismatch"
            );
            return Err("zone/epoch mismatch");
        }
        if proposal.header.parent_hash != self.parent_hash && !self.force_adopt_next_cert {
            debug!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                proposal_parent = %hex::encode(&proposal.header.parent_hash[..8]),
                local_parent = %hex::encode(&self.parent_hash[..8]),
                height = self.height,
                "rejecting proposal: parent_hash mismatch (chain divergence)"
            );
            return Err("parent_hash mismatch");
        }
        if proposal.header.parent_hash != self.parent_hash
            && self.force_adopt_next_cert
            && proposal.header.height < self.height
        {
            warn!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                proposal_height = proposal.header.height,
                local_height = self.height,
                "fork recovery: rejecting proposal that would jump backward"
            );
            return Err("fork recovery: backward jump");
        }
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == proposal.header.proposer_id)
        {
            warn!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                proposer = %hex::encode(&proposal.header.proposer_id[..8]),
                "rejecting proposal: proposer not in committee"
            );
            return Err("proposer not in committee");
        }
        Ok(())
    }

    /// Validate a proposal and produce a vote if it checks out.
    ///
    /// The caller is responsible for:
    /// 1. Verifying all spend proofs in the proposal
    /// 2. Checking nullifiers against the NullifierSet
    /// 3. Passing only valid proposals to this method
    pub fn vote_on_proposal(
        &mut self,
        proposal: &Block,
        sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Option<ConsensusAction> {
        self.validate_proposal(proposal).ok()?;

        if proposal.header.parent_hash != self.parent_hash && self.force_adopt_next_cert {
            warn!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                proposal_parent = %hex::encode(&proposal.header.parent_hash[..8]),
                local_parent = %hex::encode(&self.parent_hash[..8]),
                old_height = self.height,
                new_height = proposal.header.height,
                "fork recovery: adopting proposal's chain tip"
            );
            self.parent_hash = proposal.header.parent_hash;
            self.height = proposal.header.height;
            self.force_adopt_next_cert = false;
            self.fork_recovery_used = true;
        }

        self.proposal_seen = true;
        self.warmup_ticks = 0;
        self.voted_block_hash = Some(proposal.block_hash);
        self.cert_builder.retain_block(proposal.block_hash);

        let signature = sign_fn(&proposal.block_hash);
        let vote = BlockVote {
            zone_id: self.zone_id,
            epoch: self.epoch,
            block_hash: proposal.block_hash,
            voter_id: self.my_validator_id,
            signature,
        };

        info!(
            zone_id = self.zone_id,
            node_id = self.node_id,
            round = self.round,
            block_hash = %hex::encode(&proposal.block_hash[..8]),
            proposer = %hex::encode(&proposal.header.proposer_id[..8]),
            tx_count = proposal.transactions.len(),
            "voting on proposal"
        );

        Some(ConsensusAction::BroadcastVote(vote))
    }

    /// Process an incoming vote. Returns a certificate action if quorum is reached.
    pub fn receive_vote(&mut self, vote: BlockVote) -> Option<ConsensusAction> {
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == vote.voter_id)
        {
            warn!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                voter = %hex::encode(&vote.voter_id[..8]),
                block_hash = %hex::encode(&vote.block_hash[..8]),
                "dropping vote: voter not in committee"
            );
            return None;
        }

        if let Some(h) = self.voted_block_hash {
            if vote.block_hash != h {
                debug!(
                    zone_id = self.zone_id,
                    node_id = self.node_id,
                    round = self.round,
                    voter = %hex::encode(&vote.voter_id[..8]),
                    vote_block = %hex::encode(&vote.block_hash[..8]),
                    accepted_block = %hex::encode(&h[..8]),
                    "dropping cross-round vote: block_hash differs from accepted proposal"
                );
                return None;
            }
        }

        if self.pending_proposal.as_ref().is_some_and(|p| p.block_hash == vote.block_hash) {
            self.ticks_in_round = 0;
        }

        if let Some(cert) = self.cert_builder.add_vote(vote, self.parent_hash, self.height) {
            if cert.block_hash == self.parent_hash {
                debug!(
                    zone_id = self.zone_id,
                    node_id = self.node_id,
                    round = self.round,
                    block_hash = %hex::encode(&cert.block_hash[..8]),
                    "ignoring quorum cert: block already applied (double-advancement guard)"
                );
                return None;
            }
            info!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                height = self.height,
                block_hash = %hex::encode(&cert.block_hash[..8]),
                signers = cert.signatures.len(),
                "quorum reached, certificate produced"
            );
            self.advance_round(cert.block_hash);
            Some(ConsensusAction::BroadcastCertificate(cert))
        } else {
            None
        }
    }

    /// Try to adopt a mismatched certificate via fork recovery.
    /// Returns `Some(true)` if adopted, `Some(false)` if rejected,
    /// `None` if fork recovery is not armed.
    fn try_fork_recovery_cert(&mut self, cert: &FinalityCertificate) -> Option<bool> {
        if !self.force_adopt_next_cert {
            return None;
        }
        if cert.height + 1 < self.height {
            warn!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                cert_height = cert.height,
                local_height = self.height,
                cert_block = %hex::encode(&cert.block_hash[..8]),
                "fork recovery: rejecting ancient cert (would jump backward)"
            );
            return Some(false);
        }
        info!(
            zone_id = self.zone_id,
            node_id = self.node_id,
            round = self.round,
            cert_parent = %hex::encode(&cert.parent_hash[..8]),
            local_parent = %hex::encode(&self.parent_hash[..8]),
            cert_block = %hex::encode(&cert.block_hash[..8]),
            cert_height = cert.height,
            local_height = self.height,
            "fork recovery: adopting cert chain despite parent_hash mismatch"
        );
        self.parent_hash = cert.parent_hash;
        self.height = cert.height;
        self.force_adopt_next_cert = false;
        self.fork_recovery_used = true;
        self.advance_round_inner(cert.block_hash, false);
        Some(true)
    }

    /// Apply a received certificate (e.g. from the global topic).
    ///
    /// Accepts certs from the current epoch or the immediately previous epoch
    /// to handle the epoch-boundary race where some nodes transition before
    /// applying a late cert.
    pub fn apply_certificate(&mut self, cert: &FinalityCertificate) -> bool {
        if cert.zone_id != self.zone_id {
            return false;
        }
        if cert.block_hash == self.parent_hash {
            return true;
        }
        if cert.epoch != self.epoch && cert.epoch + 1 != self.epoch {
            debug!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                cert_epoch = cert.epoch,
                local_epoch = self.epoch,
                "skipping certificate: epoch too old"
            );
            return false;
        }

        self.warmup_ticks = 0;

        if cert.parent_hash != self.parent_hash {
            if let Some(adopted) = self.try_fork_recovery_cert(cert) {
                return adopted;
            }
            debug!(
                zone_id = self.zone_id,
                node_id = self.node_id,
                round = self.round,
                cert_parent = %hex::encode(&cert.parent_hash[..8]),
                local_parent = %hex::encode(&self.parent_hash[..8]),
                cert_block = %hex::encode(&cert.block_hash[..8]),
                height = self.height,
                "deferring certificate: parent_hash mismatch"
            );
            return false;
        }

        self.advance_round(cert.block_hash);
        true
    }

    /// Transition to the next epoch with a new committee.
    ///
    /// Resets round to 0, clears the pending proposal, and rebuilds the
    /// certificate builder for the new epoch. Height is preserved across
    /// epochs (monotonic per-zone counter).
    pub fn advance_to_epoch(&mut self, new_epoch: EpochId, new_committee: Vec<ValidatorInfo>) {
        self.epoch = new_epoch;
        self.committee = new_committee;
        self.round = 0;
        self.pending_proposal = None;
        self.rebroadcast_count = 0;
        self.ticks_in_round = 0;
        // consecutive_timeouts intentionally preserved across epochs so the
        // stall-recovery threshold can accumulate even when epoch boundaries
        // intervene.
        self.consecutive_successes = 0;
        self.proposal_seen = false;
        self.cert_builder =
            CertificateBuilder::new(self.zone_id, new_epoch, self.config.quorum_threshold);
    }

    /// Arm fork recovery: the next incoming certificate will be adopted even
    /// if its `parent_hash` doesn't match ours. This allows a stalled node to
    /// jump onto whatever chain the majority is producing.
    ///
    /// Returns `true` if newly armed, `false` if already armed (avoids log spam).
    pub fn enable_fork_recovery(&mut self) -> bool {
        if self.force_adopt_next_cert {
            return false;
        }
        self.force_adopt_next_cert = true;
        true
    }

    /// Returns `true` if fork recovery was used since the last call, and
    /// clears the flag. The service layer uses this to clear pending_certs
    /// after a fork-recovery adoption.
    pub fn take_fork_recovery_used(&mut self) -> bool {
        std::mem::take(&mut self.fork_recovery_used)
    }

    pub fn parent_hash(&self) -> &[u8; 32] {
        &self.parent_hash
    }

    pub fn consecutive_timeouts(&self) -> u32 {
        self.consecutive_timeouts
    }

    pub fn consecutive_successes(&self) -> u32 {
        self.consecutive_successes
    }

    pub fn ticks_in_round(&self) -> u32 {
        self.ticks_in_round
    }

    pub fn leader_id(&self) -> [u8; 32] {
        leader_for_round(&self.committee, self.epoch, self.round).validator_id
    }

    pub fn pending_proposal_hash(&self) -> Option<[u8; 32]> {
        self.pending_proposal.as_ref().map(|b| b.block_hash)
    }

    pub fn vote_count_for_pending(&self) -> usize {
        match self.pending_proposal.as_ref() {
            Some(b) => self.cert_builder.vote_count(&b.block_hash),
            None => 0,
        }
    }

    pub fn parent_hash_hex(&self) -> String {
        hex::encode(&self.parent_hash[..8])
    }

    pub fn committee_size(&self) -> usize {
        self.committee.len()
    }

    pub fn proposal_seen(&self) -> bool {
        self.proposal_seen
    }

    /// Number of distinct block hashes that have received at least one vote.
    pub fn vote_block_count(&self) -> usize {
        self.cert_builder.pending_count()
    }

    /// `(distinct_block_count, max_votes_for_any_single_block)`.
    pub fn vote_summary(&self) -> (usize, usize) {
        (
            self.cert_builder.pending_count(),
            self.cert_builder.max_vote_count(),
        )
    }

    fn advance_round(&mut self, new_parent_hash: [u8; 32]) {
        self.advance_round_inner(new_parent_hash, true);
    }

    fn advance_round_inner(&mut self, new_parent_hash: [u8; 32], is_genuine_progress: bool) {
        let old_height = self.height;
        self.parent_hash = new_parent_hash;
        self.round += 1;
        self.height += 1;
        self.pending_proposal = None;
        self.rebroadcast_count = 0;
        self.ticks_in_round = 0;
        if is_genuine_progress {
            self.consecutive_successes += 1;
            if self.consecutive_successes >= STALL_DECAY_SUCCESSES {
                self.consecutive_timeouts = self.consecutive_timeouts.saturating_sub(1);
                self.consecutive_successes = 0;
            }
        }
        self.proposal_seen = false;
        self.voted_block_hash = None;
        self.cert_builder.clear_votes();
        info!(
            zone_id = self.zone_id,
            node_id = self.node_id,
            old_height,
            new_height = self.height,
            parent = %hex::encode(&new_parent_hash[..8]),
            "height advanced"
        );
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
