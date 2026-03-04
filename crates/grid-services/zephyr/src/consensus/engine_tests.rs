use super::*;

pub(crate) fn make_committee(n: usize) -> Vec<ValidatorInfo> {
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

pub(crate) fn test_config() -> ZephyrConfig {
    ZephyrConfig {
        total_zones: 4,
        committee_size: 3,
        quorum_threshold: 2,
        max_block_size: 64,
        ..ZephyrConfig::default()
    }
}

pub(crate) fn identity_sign(data: &[u8]) -> Vec<u8> {
    data.to_vec()
}

#[test]
fn leader_can_propose() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let mut zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config(), 0);

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
    let mut zc = ZoneConsensus::new(0, 0, committee, non_leader_id, [0; 32], test_config(), 0);
    assert!(!zc.is_leader());
    assert!(zc.propose(vec![], identity_sign).is_none());
}

#[test]
fn re_proposal_returns_same_block() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let mut zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config(), 0);

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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

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
        1,
    );
    let vote_action = voter.vote_on_proposal(&block, identity_sign);
    assert!(vote_action.is_some());
}

#[test]
fn reject_proposal_with_wrong_head() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

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
        1,
    );
    assert!(voter.vote_on_proposal(&block, identity_sign).is_none());
}

#[test]
fn quorum_produces_certificate() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

    let block = match zc.propose(vec![], identity_sign).unwrap() {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    let mut collector =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);
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
        1,
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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);
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
        1,
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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);
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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xBB; 32], test_config(), 0);
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
        1,
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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

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
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    for _ in 0..5 {
        zc.timeout_round();
    }
    assert_eq!(zc.consecutive_timeouts(), 5);
    assert_eq!(zc.consecutive_successes(), 0);

    // First success: with STALL_DECAY_SUCCESSES=1, a single success
    // decrements consecutive_timeouts from 5 to 4 and resets successes.
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
        "first success should decrement timeouts to 4 (threshold is 1)"
    );
    assert_eq!(
        zc.consecutive_successes(),
        0,
        "consecutive_successes should reset after decay"
    );

    // Second success: decrements again 4->3.
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
        "second success should decrement timeouts to 3"
    );
    assert_eq!(
        zc.consecutive_successes(),
        0,
        "consecutive_successes should reset after decay"
    );
}

#[test]
fn fork_recovery_resets_consecutive_timeouts() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

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
        0,
        "fork recovery should reset consecutive_timeouts to 0"
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

#[test]
fn fork_recovery_accepts_cert_one_behind() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    let normal_cert = FinalityCertificate {
        zone_id: 0,
        epoch: 0,
        height: 0,
        block_hash: [0xBB; 32],
        parent_hash: [0xAA; 32],
        signatures: vec![],
    };
    assert!(zc.apply_certificate(&normal_cert));
    assert_eq!(zc.height(), 1);

    zc.timeout_round();
    zc.timeout_round();
    zc.enable_fork_recovery();

    let cert = FinalityCertificate {
        zone_id: 0,
        epoch: 0,
        height: 0,
        block_hash: [0xDD; 32],
        parent_hash: [0xCC; 32],
        signatures: vec![],
    };
    assert!(
        zc.apply_certificate(&cert),
        "cert 1-behind (cert.height+1 == local.height) should be accepted by fork recovery"
    );
    assert_eq!(zc.height(), 1, "height should stay at 1 after applying cert at height 0 + advance");
}

#[test]
fn fork_recovery_rejects_ancient_cert() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    for i in 0..5u8 {
        let cert = FinalityCertificate {
            zone_id: 0,
            epoch: 0,
            height: i as u64,
            block_hash: [0x10 + i; 32],
            parent_hash: if i == 0 { [0xAA; 32] } else { [0x10 + i - 1; 32] },
            signatures: vec![],
        };
        assert!(zc.apply_certificate(&cert));
    }
    assert_eq!(zc.height(), 5);

    zc.timeout_round();
    zc.timeout_round();
    zc.enable_fork_recovery();

    let ancient_cert = FinalityCertificate {
        zone_id: 0,
        epoch: 0,
        height: 0,
        block_hash: [0xFF; 32],
        parent_hash: [0x00; 32],
        signatures: vec![],
    };
    assert!(
        !zc.apply_certificate(&ancient_cert),
        "ancient cert (height 0 when local is 5) should be rejected"
    );
}

#[test]
fn fork_recovery_rejects_backward_proposal() {
    let committee = make_committee(3);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut leader =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xBB; 32], test_config(), 0);
    let block = match leader.propose(vec![], identity_sign).unwrap() {
        ConsensusAction::BroadcastProposal(b) => b,
        _ => panic!("expected proposal"),
    };

    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[1].validator_id,
        [0xAA; 32],
        test_config(),
        1,
    );

    let normal_cert = FinalityCertificate {
        zone_id: 0,
        epoch: 0,
        height: 0,
        block_hash: [0xCC; 32],
        parent_hash: [0xAA; 32],
        signatures: vec![],
    };
    assert!(voter.apply_certificate(&normal_cert));
    assert_eq!(voter.height(), 1);

    voter.enable_fork_recovery();
    assert!(
        voter.vote_on_proposal(&block, identity_sign).is_none(),
        "proposal at height 0 should be rejected when voter is at height 1"
    );
}
