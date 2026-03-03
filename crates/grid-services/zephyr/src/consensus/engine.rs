use grid_programs_zephyr::{
    Block, BlockVote, EpochId, FinalityCertificate, SpendTransaction, ValidatorInfo, ZoneId,
};

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
        self.ticks_in_round >= timeout_ticks
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
        &self,
        proposal: &Block,
        sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Option<ConsensusAction> {
        if proposal.header.zone_id != self.zone_id || proposal.header.epoch != self.epoch {
            return None;
        }
        if proposal.header.parent_hash != self.parent_hash {
            return None;
        }
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == proposal.header.proposer_id)
        {
            return None;
        }

        let signature = sign_fn(&proposal.block_hash);
        let vote = BlockVote {
            zone_id: self.zone_id,
            epoch: self.epoch,
            block_hash: proposal.block_hash,
            voter_id: self.my_validator_id,
            signature,
        };

        Some(ConsensusAction::BroadcastVote(vote))
    }

    /// Process an incoming vote. Returns a certificate action if quorum is reached.
    pub fn receive_vote(&mut self, vote: BlockVote) -> Option<ConsensusAction> {
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == vote.voter_id)
        {
            return None;
        }

        // A committee member produced a vote — consensus is progressing.
        // Reset the timeout so we don't discard accumulated votes while
        // quorum is still being assembled.
        self.ticks_in_round = 0;

        if let Some(cert) = self.cert_builder.add_vote(vote, self.parent_hash) {
            // Guard against double-advancement: if apply_certificate already
            // moved us forward for this block, parent_hash == cert.block_hash.
            if cert.block_hash == self.parent_hash {
                return None;
            }
            self.advance_round(cert.block_hash);
            Some(ConsensusAction::BroadcastCertificate(cert))
        } else {
            None
        }
    }

    /// Apply a received certificate (e.g. from the global topic).
    pub fn apply_certificate(&mut self, cert: &FinalityCertificate) -> bool {
        if cert.zone_id != self.zone_id || cert.epoch != self.epoch {
            return false;
        }

        if cert.parent_hash != self.parent_hash {
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
        self.cert_builder =
            CertificateBuilder::new(self.zone_id, new_epoch, self.config.quorum_threshold);
    }

    pub fn parent_hash(&self) -> &[u8; 32] {
        &self.parent_hash
    }

    fn advance_round(&mut self, new_parent_hash: [u8; 32]) {
        self.parent_hash = new_parent_hash;
        self.round += 1;
        self.height += 1;
        self.pending_proposal = None;
        self.rebroadcast_count = 0;
        self.ticks_in_round = 0;
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

        let voter = ZoneConsensus::new(
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

        let voter = ZoneConsensus::new(
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
}
