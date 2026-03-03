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
const MAX_PROPOSAL_REBROADCASTS: u32 = 2;

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
    proposal_seen: bool,
    force_adopt_next_cert: bool,
    fork_recovery_used: bool,
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
    ) -> Self {
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
            proposal_seen: false,
            force_adopt_next_cert: false,
            fork_recovery_used: false,
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

    pub fn is_leader(&self) -> bool {
        let leader = leader_for_round(&self.committee, self.epoch, self.round);
        leader.validator_id == self.my_validator_id
    }

    /// Increment the per-round tick counter.  Called once per round-timer fire.
    pub fn tick(&mut self) {
        self.ticks_in_round += 1;
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
        self.proposal_seen = false;
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
        if self.proposal_seen {
            debug!(
                zone_id = self.zone_id,
                round = self.round,
                block_hash = %hex::encode(&proposal.block_hash[..8]),
                "ignoring proposal: already voted this round"
            );
            return None;
        }
        if proposal.header.zone_id != self.zone_id || proposal.header.epoch != self.epoch {
            warn!(
                zone_id = self.zone_id,
                round = self.round,
                proposal_zone = proposal.header.zone_id,
                proposal_epoch = proposal.header.epoch,
                local_epoch = self.epoch,
                "rejecting proposal: zone/epoch mismatch"
            );
            return None;
        }
        if proposal.header.parent_hash != self.parent_hash {
            if self.force_adopt_next_cert {
                warn!(
                    zone_id = self.zone_id,
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
            } else {
                warn!(
                    zone_id = self.zone_id,
                    round = self.round,
                    proposal_parent = %hex::encode(&proposal.header.parent_hash[..8]),
                    local_parent = %hex::encode(&self.parent_hash[..8]),
                    height = self.height,
                    "rejecting proposal: parent_hash mismatch (chain divergence)"
                );
                return None;
            }
        }
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == proposal.header.proposer_id)
        {
            warn!(
                zone_id = self.zone_id,
                round = self.round,
                proposer = %hex::encode(&proposal.header.proposer_id[..8]),
                "rejecting proposal: proposer not in committee"
            );
            return None;
        }

        self.proposal_seen = true;

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
                round = self.round,
                voter = %hex::encode(&vote.voter_id[..8]),
                block_hash = %hex::encode(&vote.block_hash[..8]),
                "dropping vote: voter not in committee"
            );
            return None;
        }

        if self.pending_proposal.as_ref().is_some_and(|p| p.block_hash == vote.block_hash) {
            self.ticks_in_round = 0;
        }

        if let Some(cert) = self.cert_builder.add_vote(vote, self.parent_hash, self.height) {
            if cert.block_hash == self.parent_hash {
                debug!(
                    zone_id = self.zone_id,
                    round = self.round,
                    block_hash = %hex::encode(&cert.block_hash[..8]),
                    "ignoring quorum cert: block already applied (double-advancement guard)"
                );
                return None;
            }
            info!(
                zone_id = self.zone_id,
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

    /// Apply a received certificate (e.g. from the global topic).
    ///
    /// Accepts certs from the current epoch or the immediately previous epoch
    /// to handle the epoch-boundary race where some nodes transition before
    /// applying a late cert.
    pub fn apply_certificate(&mut self, cert: &FinalityCertificate) -> bool {
        if cert.zone_id != self.zone_id {
            return false;
        }
        if cert.epoch != self.epoch && cert.epoch + 1 != self.epoch {
            debug!(
                zone_id = self.zone_id,
                round = self.round,
                cert_zone = cert.zone_id,
                cert_epoch = cert.epoch,
                local_epoch = self.epoch,
                "skipping certificate: epoch too old"
            );
            return false;
        }

        if cert.parent_hash != self.parent_hash {
            if self.force_adopt_next_cert {
                if cert.height < self.height {
                    warn!(
                        zone_id = self.zone_id,
                        round = self.round,
                        cert_height = cert.height,
                        local_height = self.height,
                        cert_block = %hex::encode(&cert.block_hash[..8]),
                        "fork recovery: rejecting cert with height < local height"
                    );
                    return false;
                }
                warn!(
                    zone_id = self.zone_id,
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
                return true;
            }
            debug!(
                zone_id = self.zone_id,
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
        self.parent_hash = new_parent_hash;
        self.round += 1;
        self.height += 1;
        self.pending_proposal = None;
        self.rebroadcast_count = 0;
        self.ticks_in_round = 0;
        if is_genuine_progress {
            self.consecutive_timeouts = self.consecutive_timeouts.saturating_sub(1);
        }
        self.proposal_seen = false;
        self.cert_builder.clear_votes();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_committee(n: usize) -> Vec<ValidatorInfo> {
        (0..n)
            .map(|i| {
                let mut id = [0u8; 32];
                id[0] = i as u8;
                ValidatorInfo {
                    validator_id: id,
                    pubkey: id,
                    p2p_endpoint: format!("/ip4/127.0.0.1/tcp/{}", 4000 + i),
                }
            })
            .collect()
    }

    fn test_config() -> ZephyrConfig {
        ZephyrConfig {
            total_zones: 4,
            committee_size: 3,
            quorum_threshold: 2,
            max_block_size: 64,
            ..ZephyrConfig::default()
        }
    }

    fn identity_sign(data: &[u8]) -> Vec<u8> {
        data.to_vec()
    }

    #[test]
    fn leader_can_propose() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config());

        assert!(zc.is_leader());
        let action = zc.propose(vec![], identity_sign);
        assert!(action.is_some());
    }

    #[test]
    fn non_leader_cannot_propose() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut non_leader_id = [0u8; 32];
        for v in &committee {
            if v.validator_id != leader_id {
                non_leader_id = v.validator_id;
                break;
            }
        }
        let mut zc = ZoneConsensus::new(0, 0, committee, non_leader_id, [0; 32], test_config());
        assert!(!zc.is_leader());
        assert!(zc.propose(vec![], identity_sign).is_none());
    }

    #[test]
    fn re_proposal_returns_same_block() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config());

        let first = zc.propose(vec![], identity_sign).unwrap();
        let second = zc.propose(vec![], identity_sign).unwrap();
        let hash1 = match first {
            ConsensusAction::BroadcastProposal(b) => b.block_hash,
            _ => panic!("expected proposal"),
        };
        let hash2 = match second {
            ConsensusAction::BroadcastProposal(b) => b.block_hash,
            _ => panic!("expected proposal"),
        };
        assert_eq!(hash1, hash2, "re-proposal must return the same block");
    }

    #[test]
    fn vote_on_valid_proposal() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        let action = zc.propose(vec![], identity_sign).unwrap();
        let block = match action {
            ConsensusAction::BroadcastProposal(p) => p,
            _ => panic!("expected proposal"),
        };

        let mut voter = ZoneConsensus::new(
            0,
            0,
            committee.clone(),
            committee[1].validator_id,
            [0; 32],
            test_config(),
        );
        let vote_action = voter.vote_on_proposal(&block, identity_sign);
        assert!(vote_action.is_some());
    }

    #[test]
    fn reject_proposal_with_wrong_head() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        let action = zc.propose(vec![], identity_sign).unwrap();
        let block = match action {
            ConsensusAction::BroadcastProposal(p) => p,
            _ => panic!("expected proposal"),
        };

        let mut voter = ZoneConsensus::new(
            0,
            0,
            committee.clone(),
            committee[1].validator_id,
            [0xFF; 32],
            test_config(),
        );
        assert!(voter.vote_on_proposal(&block, identity_sign).is_none());
    }

    #[test]
    fn quorum_produces_certificate() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        let block = match zc.propose(vec![], identity_sign).unwrap() {
            ConsensusAction::BroadcastProposal(p) => p,
            _ => panic!("expected proposal"),
        };

        let mut collector =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        for voter in &committee[..2] {
            let vote = BlockVote {
                zone_id: 0,
                epoch: 0,
                block_hash: block.block_hash,
                voter_id: voter.validator_id,
                signature: block.block_hash.to_vec(),
            };
            if let Some(ConsensusAction::BroadcastCertificate(cert)) = collector.receive_vote(vote)
            {
                assert_eq!(cert.zone_id, 0);
                assert_eq!(cert.signatures.len(), 2);
                return;
            }
        }
        panic!("expected certificate after quorum");
    }

    #[test]
    fn second_proposal_rejected_after_voting() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut leader =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());
        let block_a = match leader.propose(vec![], identity_sign).unwrap() {
            ConsensusAction::BroadcastProposal(b) => b,
            _ => panic!("expected proposal"),
        };

        let mut voter = ZoneConsensus::new(
            0,
            0,
            committee.clone(),
            committee[1].validator_id,
            [0; 32],
            test_config(),
        );

        let first_vote = voter.vote_on_proposal(&block_a, identity_sign);
        assert!(first_vote.is_some(), "first vote should succeed");
        assert!(voter.proposal_seen(), "proposal_seen should be set");

        let second_vote = voter.vote_on_proposal(&block_a, identity_sign);
        assert!(
            second_vote.is_none(),
            "second vote in same round must be rejected"
        );
    }

    #[test]
    fn vote_lock_resets_after_timeout() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut leader =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());
        let block = match leader.propose(vec![], identity_sign).unwrap() {
            ConsensusAction::BroadcastProposal(b) => b,
            _ => panic!("expected proposal"),
        };

        let mut voter = ZoneConsensus::new(
            0,
            0,
            committee.clone(),
            committee[1].validator_id,
            [0; 32],
            test_config(),
        );

        voter.vote_on_proposal(&block, identity_sign);
        assert!(voter.proposal_seen());

        voter.timeout_round();
        assert!(!voter.proposal_seen(), "proposal_seen should reset on timeout");
    }

    #[test]
    fn leader_can_self_vote_after_propose() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());
        let block = match zc.propose(vec![], identity_sign).unwrap() {
            ConsensusAction::BroadcastProposal(b) => b,
            _ => panic!("expected proposal"),
        };

        let vote = zc.vote_on_proposal(&block, identity_sign);
        assert!(
            vote.is_some(),
            "leader must be able to self-vote after proposing"
        );
    }

    #[test]
    fn apply_cert_from_previous_epoch() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config());

        zc.advance_to_epoch(1, committee.clone());
        assert_eq!(zc.epoch(), 1);

        let cert = FinalityCertificate {
            zone_id: 0,
            epoch: 0, // previous epoch
            height: 0,
            block_hash: [0xBB; 32],
            parent_hash: [0xAA; 32],
            signatures: vec![],
        };
        assert!(
            zc.apply_certificate(&cert),
            "cert from epoch N-1 should be accepted when engine is at epoch N"
        );
        assert_eq!(zc.parent_hash(), &[0xBB; 32]);
    }

    #[test]
    fn fork_recovery_adopts_mismatched_cert() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config());

        let cert = FinalityCertificate {
            zone_id: 0,
            epoch: 0,
            height: 0,
            block_hash: [0xCC; 32],
            parent_hash: [0xBB; 32], // doesn't match our [0xAA; 32]
            signatures: vec![],
        };
        assert!(
            !zc.apply_certificate(&cert),
            "cert with mismatched parent should be rejected normally"
        );

        zc.enable_fork_recovery();
        assert!(
            zc.apply_certificate(&cert),
            "cert should be adopted after fork recovery is enabled"
        );
        assert_eq!(zc.parent_hash(), &[0xCC; 32]);

        let cert2 = FinalityCertificate {
            zone_id: 0,
            epoch: 0,
            height: 1,
            block_hash: [0xDD; 32],
            parent_hash: [0xFF; 32],
            signatures: vec![],
        };
        assert!(
            !zc.apply_certificate(&cert2),
            "force_adopt flag should be consumed after one use"
        );
    }

    #[test]
    fn fork_recovery_adopts_proposal_chain_tip() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut leader =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xBB; 32], test_config());
        let block = match leader.propose(vec![], identity_sign).unwrap() {
            ConsensusAction::BroadcastProposal(b) => b,
            _ => panic!("expected proposal"),
        };
        assert_eq!(block.header.parent_hash, [0xBB; 32]);

        let mut voter = ZoneConsensus::new(
            0,
            0,
            committee.clone(),
            committee[1].validator_id,
            [0xAA; 32], // different chain tip
            test_config(),
        );

        assert!(
            voter.vote_on_proposal(&block, identity_sign).is_none(),
            "proposal with mismatched parent should be rejected normally"
        );

        voter.enable_fork_recovery();
        let vote = voter.vote_on_proposal(&block, identity_sign);
        assert!(
            vote.is_some(),
            "proposal should be accepted after fork recovery is enabled"
        );
        assert_eq!(
            voter.parent_hash(),
            &[0xBB; 32],
            "voter should adopt the proposal's parent_hash"
        );
        assert_eq!(
            voter.height(),
            block.header.height,
            "voter should adopt the proposal's height"
        );
    }

    #[test]
    fn consecutive_timeouts_persist_across_epochs() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        zc.timeout_round();
        zc.timeout_round();
        zc.timeout_round();
        assert_eq!(zc.consecutive_timeouts(), 3);

        zc.advance_to_epoch(1, committee.clone());
        assert_eq!(
            zc.consecutive_timeouts(),
            3,
            "consecutive_timeouts must survive epoch transitions"
        );
    }

    #[test]
    fn advance_round_decrements_consecutive_timeouts() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config());

        for _ in 0..5 {
            zc.timeout_round();
        }
        assert_eq!(zc.consecutive_timeouts(), 5);

        let cert = FinalityCertificate {
            zone_id: 0,
            epoch: 0,
            height: 0,
            block_hash: [0xBB; 32],
            parent_hash: [0xAA; 32],
            signatures: vec![],
        };
        assert!(zc.apply_certificate(&cert));
        assert_eq!(
            zc.consecutive_timeouts(),
            4,
            "advance_round should decrement by 1, not zero"
        );

        let cert2 = FinalityCertificate {
            zone_id: 0,
            epoch: 0,
            height: 1,
            block_hash: [0xCC; 32],
            parent_hash: [0xBB; 32],
            signatures: vec![],
        };
        assert!(zc.apply_certificate(&cert2));
        assert_eq!(
            zc.consecutive_timeouts(),
            3,
            "second advance_round should decrement to 3"
        );
    }

    #[test]
    fn fork_recovery_does_not_decrement_consecutive_timeouts() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;

        let mut zc =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config());

        for _ in 0..4 {
            zc.timeout_round();
        }
        assert_eq!(zc.consecutive_timeouts(), 4);

        zc.enable_fork_recovery();

        let cert = FinalityCertificate {
            zone_id: 0,
            epoch: 0,
            height: 5,
            block_hash: [0xCC; 32],
            parent_hash: [0xBB; 32],
            signatures: vec![],
        };
        assert!(zc.apply_certificate(&cert));
        assert_eq!(
            zc.consecutive_timeouts(),
            4,
            "fork recovery should NOT decrement consecutive_timeouts"
        );
        assert!(
            zc.take_fork_recovery_used(),
            "fork_recovery_used flag should be set"
        );
        assert!(
            !zc.take_fork_recovery_used(),
            "flag should be cleared after take"
        );
    }
}
