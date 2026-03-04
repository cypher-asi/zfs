use super::*;
use ed25519_dalek::{Signer, SigningKey};
use grid_programs_zephyr::CertSignature;

fn make_keys(n: usize) -> Vec<SigningKey> {
    (0..n)
        .map(|i| {
            let mut seed = [0u8; 32];
            seed[0] = i as u8;
            SigningKey::from_bytes(&seed)
        })
        .collect()
}

fn make_committee_from_keys(keys: &[SigningKey]) -> Vec<ValidatorInfo> {
    keys.iter()
        .enumerate()
        .map(|(i, sk)| {
            let pk = sk.verifying_key().to_bytes();
            ValidatorInfo {
                validator_id: pk,
                pubkey: pk,
                p2p_endpoint: format!("/ip4/127.0.0.1/tcp/{}", 4000 + i),
            }
        })
        .collect()
}

fn make_sign_fn(key: &SigningKey) -> impl FnOnce(&[u8]) -> Vec<u8> {
    let key = key.clone();
    move |data: &[u8]| key.sign(data).to_bytes().to_vec()
}

fn key_index_for_id(keys: &[SigningKey], id: &[u8; 32]) -> usize {
    keys.iter()
        .position(|k| k.verifying_key().to_bytes() == *id)
        .expect("validator_id not found in keys")
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

fn make_cert(
    zone_id: ZoneId,
    epoch: EpochId,
    height: u64,
    block_hash: [u8; 32],
    parent_hash: [u8; 32],
    keys: &[SigningKey],
    signer_indices: &[usize],
) -> FinalityCertificate {
    FinalityCertificate {
        zone_id,
        epoch,
        height,
        block_hash,
        parent_hash,
        signatures: signer_indices
            .iter()
            .map(|&i| CertSignature {
                validator_id: keys[i].verifying_key().to_bytes(),
                signature: keys[i].sign(&block_hash).to_bytes().to_vec(),
            })
            .collect(),
    }
}

#[test]
fn leader_can_propose() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);
    let mut zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config(), 0);

    assert!(zc.is_leader());
    let action = zc.propose(vec![], make_sign_fn(&keys[li]));
    assert!(action.is_some());
}

#[test]
fn non_leader_cannot_propose() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let non_leader_id = committee
        .iter()
        .find(|v| v.validator_id != leader_id)
        .unwrap()
        .validator_id;
    let ni = key_index_for_id(&keys, &non_leader_id);
    let mut zc = ZoneConsensus::new(0, 0, committee, non_leader_id, [0; 32], test_config(), 0);
    assert!(!zc.is_leader());
    assert!(zc.propose(vec![], make_sign_fn(&keys[ni])).is_none());
}

#[test]
fn re_proposal_returns_same_block() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);
    let mut zc = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], test_config(), 0);

    let first = zc.propose(vec![], make_sign_fn(&keys[li])).unwrap();
    for _ in 0..4 {
        zc.tick();
    }
    let second = zc.propose(vec![], make_sign_fn(&keys[li])).unwrap();
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
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);
    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

    let action = zc.propose(vec![], make_sign_fn(&keys[li])).unwrap();
    let block = match action {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    let voter_idx = if li == 0 { 1 } else { 0 };
    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[voter_idx].validator_id,
        [0; 32],
        test_config(),
        1,
    );
    let vote_action = voter.vote_on_proposal(&block, make_sign_fn(&keys[voter_idx]));
    assert!(vote_action.is_some());
}

#[test]
fn reject_proposal_with_wrong_head() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);
    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

    let action = zc.propose(vec![], make_sign_fn(&keys[li])).unwrap();
    let block = match action {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    let voter_idx = if li == 0 { 1 } else { 0 };
    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[voter_idx].validator_id,
        [0xFF; 32],
        test_config(),
        1,
    );
    assert!(voter
        .vote_on_proposal(&block, make_sign_fn(&keys[voter_idx]))
        .is_none());
}

#[test]
fn quorum_produces_certificate() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);
    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

    let block = match zc.propose(vec![], make_sign_fn(&keys[li])).unwrap() {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    let mut collector =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);

    for (i, voter) in committee[..2].iter().enumerate() {
        let vote = BlockVote {
            zone_id: 0,
            epoch: 0,
            block_hash: block.block_hash,
            voter_id: voter.validator_id,
            signature: keys[i].sign(&block.block_hash).to_bytes().to_vec(),
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
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);

    let mut leader =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);
    let block_a = match leader.propose(vec![], make_sign_fn(&keys[li])).unwrap() {
        ConsensusAction::BroadcastProposal(b) => b,
        _ => panic!("expected proposal"),
    };

    let voter_idx = if li == 0 { 1 } else { 0 };
    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[voter_idx].validator_id,
        [0; 32],
        test_config(),
        1,
    );

    let first_vote = voter.vote_on_proposal(&block_a, make_sign_fn(&keys[voter_idx]));
    assert!(first_vote.is_some(), "first vote should succeed");
    assert!(voter.proposal_seen(), "proposal_seen should be set");

    let second_vote = voter.vote_on_proposal(&block_a, make_sign_fn(&keys[voter_idx]));
    assert!(
        second_vote.is_none(),
        "second vote in same round must be rejected"
    );
}

#[test]
fn vote_lock_resets_after_timeout() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);

    let mut leader =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);
    let block = match leader.propose(vec![], make_sign_fn(&keys[li])).unwrap() {
        ConsensusAction::BroadcastProposal(b) => b,
        _ => panic!("expected proposal"),
    };

    let voter_idx = if li == 0 { 1 } else { 0 };
    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[voter_idx].validator_id,
        [0; 32],
        test_config(),
        1,
    );

    voter.vote_on_proposal(&block, make_sign_fn(&keys[voter_idx]));
    assert!(voter.proposal_seen());

    voter.timeout_round();
    assert!(
        !voter.proposal_seen(),
        "proposal_seen should reset on timeout"
    );
}

#[test]
fn leader_can_self_vote_after_propose() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0; 32], test_config(), 0);
    let block = match zc.propose(vec![], make_sign_fn(&keys[li])).unwrap() {
        ConsensusAction::BroadcastProposal(b) => b,
        _ => panic!("expected proposal"),
    };

    let vote = zc.vote_on_proposal(&block, make_sign_fn(&keys[li]));
    assert!(
        vote.is_some(),
        "leader must be able to self-vote after proposing"
    );
}

#[test]
fn apply_cert_from_previous_epoch() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    zc.advance_to_epoch(1, committee.clone());
    assert_eq!(zc.epoch(), 1);

    let cert = make_cert(0, 0, 0, [0xBB; 32], [0xAA; 32], &keys, &[0, 1]);
    assert!(
        zc.apply_certificate(&cert),
        "cert from epoch N-1 should be accepted when engine is at epoch N"
    );
    assert_eq!(zc.parent_hash(), &[0xBB; 32]);
}

#[test]
fn fork_recovery_adopts_mismatched_cert() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    let cert = make_cert(0, 0, 0, [0xCC; 32], [0xBB; 32], &keys, &[0, 1]);
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

    let cert2 = make_cert(0, 0, 1, [0xDD; 32], [0xFF; 32], &keys, &[0, 1]);
    assert!(
        !zc.apply_certificate(&cert2),
        "force_adopt flag should be consumed after one use"
    );
}

#[test]
fn fork_recovery_adopts_proposal_chain_tip() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);

    let mut leader =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xBB; 32], test_config(), 0);
    let block = match leader.propose(vec![], make_sign_fn(&keys[li])).unwrap() {
        ConsensusAction::BroadcastProposal(b) => b,
        _ => panic!("expected proposal"),
    };
    assert_eq!(block.header.parent_hash, [0xBB; 32]);

    let voter_idx = if li == 0 { 1 } else { 0 };
    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[voter_idx].validator_id,
        [0xAA; 32],
        test_config(),
        1,
    );

    assert!(
        voter
            .vote_on_proposal(&block, make_sign_fn(&keys[voter_idx]))
            .is_none(),
        "proposal with mismatched parent should be rejected normally"
    );

    voter.enable_fork_recovery();
    let vote = voter.vote_on_proposal(&block, make_sign_fn(&keys[voter_idx]));
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
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
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
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    for _ in 0..5 {
        zc.timeout_round();
    }
    assert_eq!(zc.consecutive_timeouts(), 5);
    assert_eq!(zc.consecutive_successes(), 0);

    let mut parent = [0xAA; 32];
    for i in 0..7u64 {
        let mut block_hash = [0u8; 32];
        block_hash[0..8].copy_from_slice(&(i + 1).to_le_bytes());
        let cert = make_cert(0, 0, i, block_hash, parent, &keys, &[0, 1]);
        assert!(zc.apply_certificate(&cert));
        parent = block_hash;
        if i < 6 {
            assert_eq!(
                zc.consecutive_timeouts(),
                5,
                "timeouts should stay at 5 until decay threshold ({} successes so far)",
                i + 1
            );
        }
    }
    assert_eq!(
        zc.consecutive_timeouts(),
        4,
        "7 consecutive successes should decrement timeouts to 4"
    );
    assert_eq!(
        zc.consecutive_successes(),
        0,
        "consecutive_successes should reset after decay"
    );
}

#[test]
fn fork_recovery_resets_consecutive_timeouts() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    for _ in 0..4 {
        zc.timeout_round();
    }
    assert_eq!(zc.consecutive_timeouts(), 4);

    zc.enable_fork_recovery();

    let cert = make_cert(0, 0, 5, [0xCC; 32], [0xBB; 32], &keys, &[0, 1]);
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
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    let normal_cert = make_cert(0, 0, 0, [0xBB; 32], [0xAA; 32], &keys, &[0, 1]);
    assert!(zc.apply_certificate(&normal_cert));
    assert_eq!(zc.height(), 1);

    zc.timeout_round();
    zc.timeout_round();
    zc.enable_fork_recovery();

    let cert = make_cert(0, 0, 0, [0xDD; 32], [0xCC; 32], &keys, &[0, 1]);
    assert!(
        zc.apply_certificate(&cert),
        "cert 1-behind (cert.height+1 == local.height) should be accepted by fork recovery"
    );
    assert_eq!(
        zc.height(),
        1,
        "height should stay at 1 after applying cert at height 0 + advance"
    );
}

#[test]
fn fork_recovery_rejects_ancient_cert() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let mut zc =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xAA; 32], test_config(), 0);

    for i in 0..5u8 {
        let parent = if i == 0 {
            [0xAA; 32]
        } else {
            [0x10 + i - 1; 32]
        };
        let cert = make_cert(0, 0, i as u64, [0x10 + i; 32], parent, &keys, &[0, 1]);
        assert!(zc.apply_certificate(&cert));
    }
    assert_eq!(zc.height(), 5);

    zc.timeout_round();
    zc.timeout_round();
    zc.enable_fork_recovery();

    let ancient_cert = make_cert(0, 0, 0, [0xFF; 32], [0x00; 32], &keys, &[0, 1]);
    assert!(
        !zc.apply_certificate(&ancient_cert),
        "ancient cert (height 0 when local is 5) should be rejected"
    );
}

#[test]
fn fork_recovery_rejects_backward_proposal() {
    let keys = make_keys(3);
    let committee = make_committee_from_keys(&keys);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;
    let li = key_index_for_id(&keys, &leader_id);

    let mut leader =
        ZoneConsensus::new(0, 0, committee.clone(), leader_id, [0xBB; 32], test_config(), 0);
    let block = match leader.propose(vec![], make_sign_fn(&keys[li])).unwrap() {
        ConsensusAction::BroadcastProposal(b) => b,
        _ => panic!("expected proposal"),
    };

    let voter_idx = if li == 0 { 1 } else { 0 };
    let mut voter = ZoneConsensus::new(
        0,
        0,
        committee.clone(),
        committee[voter_idx].validator_id,
        [0xAA; 32],
        test_config(),
        1,
    );

    let normal_cert = make_cert(0, 0, 0, [0xCC; 32], [0xAA; 32], &keys, &[0, 1]);
    assert!(voter.apply_certificate(&normal_cert));
    assert_eq!(voter.height(), 1);

    voter.enable_fork_recovery();
    assert!(
        voter
            .vote_on_proposal(&block, make_sign_fn(&keys[voter_idx]))
            .is_none(),
        "proposal at height 0 should be rejected when voter is at height 1"
    );
}
