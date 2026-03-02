use grid_programs_zephyr::{
    BatchProposal, BatchVote, EpochId, FinalityCertificate, SpendTransaction, ValidatorInfo, ZoneId,
};

use super::leader::leader_for_round;
use super::proposal::build_batch_proposal;
use super::vote::{compute_new_zone_head, CertificateBuilder};
use crate::config::ZephyrConfig;

/// Per-zone consensus state machine.
///
/// Each zone the validator is assigned to gets an independent `ZoneConsensus`
/// instance. It tracks the current round, collects votes, and coordinates
/// proposal/certification.
pub struct ZoneConsensus {
    zone_id: ZoneId,
    epoch: EpochId,
    round: u64,
    committee: Vec<ValidatorInfo>,
    my_validator_id: [u8; 32],
    cert_builder: CertificateBuilder,
    prev_zone_head: [u8; 32],
    config: ZephyrConfig,
}

/// Actions the consensus engine requests the caller to perform.
#[derive(Debug)]
pub enum ConsensusAction {
    BroadcastProposal(BatchProposal),
    BroadcastVote(BatchVote),
    BroadcastCertificate(FinalityCertificate),
}

impl ZoneConsensus {
    pub fn new(
        zone_id: ZoneId,
        epoch: EpochId,
        committee: Vec<ValidatorInfo>,
        my_validator_id: [u8; 32],
        prev_zone_head: [u8; 32],
        config: ZephyrConfig,
    ) -> Self {
        Self {
            zone_id,
            epoch,
            round: 0,
            committee: committee.clone(),
            my_validator_id,
            cert_builder: CertificateBuilder::new(zone_id, epoch, config.quorum_threshold),
            prev_zone_head,
            config,
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

    pub fn is_leader(&self) -> bool {
        let leader = leader_for_round(&self.committee, self.epoch, self.round);
        leader.validator_id == self.my_validator_id
    }

    /// Called by the leader when the round timer fires.
    /// Takes verified spends from the mempool and builds a proposal.
    pub fn propose(
        &self,
        spends: Vec<SpendTransaction>,
        sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Option<ConsensusAction> {
        if !self.is_leader() {
            return None;
        }

        let max = self.config.max_batch_size.min(spends.len());
        let batch_spends = spends.into_iter().take(max).collect();

        let proposal = build_batch_proposal(
            self.zone_id,
            self.epoch,
            self.prev_zone_head,
            batch_spends,
            self.my_validator_id,
            sign_fn,
        );

        Some(ConsensusAction::BroadcastProposal(proposal))
    }

    /// Validate a proposal and produce a vote if it checks out.
    ///
    /// The caller is responsible for:
    /// 1. Verifying all spend proofs in the proposal
    /// 2. Checking nullifiers against the NullifierSet
    /// 3. Passing only valid proposals to this method
    pub fn vote_on_proposal(
        &self,
        proposal: &BatchProposal,
        sign_fn: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Option<ConsensusAction> {
        if proposal.zone_id != self.zone_id || proposal.epoch != self.epoch {
            return None;
        }
        if proposal.prev_zone_head != self.prev_zone_head {
            return None;
        }
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == proposal.proposer_id)
        {
            return None;
        }

        let signature = sign_fn(&proposal.batch_hash);
        let vote = BatchVote {
            zone_id: self.zone_id,
            epoch: self.epoch,
            batch_hash: proposal.batch_hash,
            voter_id: self.my_validator_id,
            signature,
        };

        Some(ConsensusAction::BroadcastVote(vote))
    }

    /// Process an incoming vote. Returns a certificate action if quorum is reached.
    pub fn receive_vote(&mut self, vote: BatchVote) -> Option<ConsensusAction> {
        if !self
            .committee
            .iter()
            .any(|v| v.validator_id == vote.voter_id)
        {
            return None;
        }

        if let Some(cert) = self.cert_builder.add_vote(vote, self.prev_zone_head) {
            self.prev_zone_head = cert.new_zone_head;
            self.round += 1;
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

        let expected_head = compute_new_zone_head(&cert.batch_hash, &cert.prev_zone_head);
        if expected_head != cert.new_zone_head {
            return false;
        }

        self.prev_zone_head = cert.new_zone_head;
        self.round += 1;
        true
    }

    pub fn prev_zone_head(&self) -> &[u8; 32] {
        &self.prev_zone_head
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
            max_batch_size: 64,
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
        let zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config());

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
        let zc = ZoneConsensus::new(0, 0, committee, non_leader_id, [0; 32], test_config());
        assert!(!zc.is_leader());
        assert!(zc.propose(vec![], identity_sign).is_none());
    }

    #[test]
    fn vote_on_valid_proposal() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let zc = ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        let action = zc.propose(vec![], identity_sign).unwrap();
        let proposal = match action {
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
        let vote_action = voter.vote_on_proposal(&proposal, identity_sign);
        assert!(vote_action.is_some());
    }

    #[test]
    fn reject_proposal_with_wrong_head() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let zc = ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        let action = zc.propose(vec![], identity_sign).unwrap();
        let proposal = match action {
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
        assert!(voter.vote_on_proposal(&proposal, identity_sign).is_none());
    }

    #[test]
    fn quorum_produces_certificate() {
        let committee = make_committee(3);
        let leader_id = leader_for_round(&committee, 0, 0).validator_id;
        let zc = ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        let proposal = match zc.propose(vec![], identity_sign).unwrap() {
            ConsensusAction::BroadcastProposal(p) => p,
            _ => panic!("expected proposal"),
        };

        let mut collector =
            ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config());

        for voter in &committee[..2] {
            let vote = BatchVote {
                zone_id: 0,
                epoch: 0,
                batch_hash: proposal.batch_hash,
                voter_id: voter.validator_id,
                signature: proposal.batch_hash.to_vec(),
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
