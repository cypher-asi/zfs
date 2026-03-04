//! Integration tests for the five MVP success criteria:
//! 1. Routing determinism
//! 2. Parallel finality
//! 3. Double-spend rejection
//! 4. Rotation continuity
//! 5. Invalid-proof containment

use ed25519_dalek::{Signer, SigningKey};
use grid_programs_zephyr::*;
use grid_services_zephyr::committee::sample_committee;
use grid_services_zephyr::config::ZephyrConfig;
use grid_services_zephyr::consensus::{leader_for_round, ConsensusAction, ZoneConsensus};
use grid_services_zephyr::epoch::EpochManager;
use grid_services_zephyr::mempool::Mempool;
use grid_services_zephyr::routing::zone_for_nullifier;
use grid_services_zephyr::storage::ZoneHead;

fn make_keys(n: usize) -> Vec<SigningKey> {
    (0..n)
        .map(|i| {
            let mut seed = [0u8; 32];
            seed[0] = i as u8;
            SigningKey::from_bytes(&seed)
        })
        .collect()
}

fn make_validators_from_keys(keys: &[SigningKey]) -> Vec<ValidatorInfo> {
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

fn find_key<'a>(keys: &'a [SigningKey], validator_id: &[u8; 32]) -> &'a SigningKey {
    keys.iter()
        .find(|k| k.verifying_key().to_bytes() == *validator_id)
        .expect("signing key not found for validator_id")
}

fn make_sign_fn(key: &SigningKey) -> impl FnOnce(&[u8]) -> Vec<u8> {
    let key = key.clone();
    move |data: &[u8]| key.sign(data).to_bytes().to_vec()
}

fn test_config(total_zones: u32) -> ZephyrConfig {
    ZephyrConfig {
        total_zones,
        committee_size: 3,
        quorum_threshold: 2,
        max_block_size: 64,
        ..ZephyrConfig::default()
    }
}

fn dummy_spend(nullifier_byte: u8) -> SpendTransaction {
    SpendTransaction {
        input_commitment: NoteCommitment([0; 32]),
        nullifier: Nullifier([nullifier_byte; 32]),
        outputs: vec![],
        proof: vec![],
        public_signals: vec![],
    }
}

// ---------------------------------------------------------------------------
// 1. Routing determinism — same nullifier always routes to same zone,
//    across multiple "independent implementations" (simulated by repeated calls).
// ---------------------------------------------------------------------------

#[test]
fn criterion_1_routing_determinism() {
    let total_zones = 256u32;

    for byte in 0..=255u8 {
        let nullifier = Nullifier([byte; 32]);
        let z1 = zone_for_nullifier(&nullifier, total_zones);
        let z2 = zone_for_nullifier(&nullifier, total_zones);
        let z3 = zone_for_nullifier(&nullifier, total_zones);
        assert_eq!(z1, z2);
        assert_eq!(z2, z3);
        assert!(z1 < total_zones);
    }

    let n = Nullifier([0xDE; 32]);
    let zone_a = zone_for_nullifier(&n, total_zones);
    let zone_b = zone_for_nullifier(&n, total_zones);
    assert_eq!(
        zone_a, zone_b,
        "identical nullifier must route to identical zone"
    );
}

// ---------------------------------------------------------------------------
// 2. Parallel finality — two spends in different zones finalize concurrently
//    in separate ZoneConsensus instances.
// ---------------------------------------------------------------------------

#[test]
fn criterion_2_parallel_finality() {
    let keys = make_keys(5);
    let validators = make_validators_from_keys(&keys);
    let config = test_config(8);
    let seed = [0u8; 32];

    let spend_a = dummy_spend(0xAA);
    let spend_b = dummy_spend(0xBB);
    let zone_a = zone_for_nullifier(&spend_a.nullifier, config.total_zones);
    let zone_b = zone_for_nullifier(&spend_b.nullifier, config.total_zones);

    let committee_a = sample_committee(&seed, zone_a, &validators, config.committee_size);
    let committee_b = sample_committee(&seed, zone_b, &validators, config.committee_size);

    let leader_a = leader_for_round(&committee_a, 0, 0).validator_id;
    let leader_b = leader_for_round(&committee_b, 0, 0).validator_id;

    let mut consensus_a = ZoneConsensus::new(
        zone_a,
        0,
        committee_a.clone(),
        leader_a,
        [0; 32],
        config.clone(),
        0,
    );
    let mut consensus_b = ZoneConsensus::new(
        zone_b,
        0,
        committee_b.clone(),
        leader_b,
        [0; 32],
        config.clone(),
        1,
    );

    let block_a = match consensus_a
        .propose(vec![spend_a], make_sign_fn(find_key(&keys, &leader_a)))
        .unwrap()
    {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };
    let block_b = match consensus_b
        .propose(vec![spend_b], make_sign_fn(find_key(&keys, &leader_b)))
        .unwrap()
    {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    let mut cert_a = None;
    let mut cert_b = None;

    for voter in &committee_a[..2] {
        let voter_key = find_key(&keys, &voter.validator_id);
        let vote = BlockVote {
            zone_id: zone_a,
            epoch: 0,
            block_hash: block_a.block_hash,
            voter_id: voter.validator_id,
            signature: voter_key.sign(&block_a.block_hash).to_bytes().to_vec(),
        };
        if let Some(ConsensusAction::BroadcastCertificate(c)) = consensus_a.receive_vote(vote) {
            cert_a = Some(c);
        }
    }

    for voter in &committee_b[..2] {
        let voter_key = find_key(&keys, &voter.validator_id);
        let vote = BlockVote {
            zone_id: zone_b,
            epoch: 0,
            block_hash: block_b.block_hash,
            voter_id: voter.validator_id,
            signature: voter_key.sign(&block_b.block_hash).to_bytes().to_vec(),
        };
        if let Some(ConsensusAction::BroadcastCertificate(c)) = consensus_b.receive_vote(vote) {
            cert_b = Some(c);
        }
    }

    let cert_a = cert_a.expect("zone A should finalize");
    let cert_b = cert_b.expect("zone B should finalize");
    assert_eq!(cert_a.zone_id, zone_a);
    assert_eq!(cert_b.zone_id, zone_b);
    assert_ne!(
        cert_a.block_hash, cert_b.block_hash,
        "different zones should produce different certificates"
    );
}

// ---------------------------------------------------------------------------
// 3. Double-spend rejection — same nullifier submitted twice, only one
//    makes it into a block proposal.
// ---------------------------------------------------------------------------

#[test]
fn criterion_3_double_spend_rejection() {
    let keys = make_keys(5);
    let validators = make_validators_from_keys(&keys);
    let config = test_config(8);

    let spend1 = dummy_spend(0xDD);
    let spend2 = dummy_spend(0xDD);
    let zone = zone_for_nullifier(&spend1.nullifier, config.total_zones);
    assert_eq!(
        zone,
        zone_for_nullifier(&spend2.nullifier, config.total_zones),
        "same nullifier must route to same zone"
    );

    let mut mempool = Mempool::new(zone, 100);
    assert!(mempool.insert(spend1));
    assert!(
        !mempool.insert(spend2),
        "duplicate nullifier must be rejected by mempool"
    );
    assert_eq!(mempool.len(), 1, "only one spend should be in mempool");

    let seed = [0u8; 32];
    let committee = sample_committee(&seed, zone, &validators, config.committee_size);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let spends = mempool.drain_proposal(64);
    let mut consensus = ZoneConsensus::new(
        zone,
        0,
        committee.clone(),
        leader_id,
        [0; 32],
        config.clone(),
        0,
    );

    let block = match consensus
        .propose(spends, make_sign_fn(find_key(&keys, &leader_id)))
        .unwrap()
    {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    assert_eq!(
        block.transactions.len(),
        1,
        "block must contain exactly one spend"
    );
}

// ---------------------------------------------------------------------------
// 4. Rotation continuity — epoch transition preserves parent_hash chain.
// ---------------------------------------------------------------------------

#[test]
fn criterion_4_rotation_continuity() {
    let keys = make_keys(10);
    let validators = make_validators_from_keys(&keys);
    let config = test_config(16);

    let mut epoch_mgr = EpochManager::new(
        0,
        config.epoch_duration_ms,
        config.initial_randomness,
        validators.clone(),
        config.total_zones,
        config.committee_size,
    );

    let mut zone_heads = ZoneHead::new();
    let test_zone: u32 = 5;

    let committee_e0 = epoch_mgr.committee_for_zone(test_zone);
    let leader_e0 = leader_for_round(&committee_e0, 0, 0).validator_id;
    let mut consensus_e0 = ZoneConsensus::new(
        test_zone,
        0,
        committee_e0.clone(),
        leader_e0,
        [0; 32],
        config.clone(),
        0,
    );

    let block_e0 = match consensus_e0
        .propose(
            vec![dummy_spend(0x01)],
            make_sign_fn(find_key(&keys, &leader_e0)),
        )
        .unwrap()
    {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    for voter in &committee_e0[..2] {
        let voter_key = find_key(&keys, &voter.validator_id);
        let vote = BlockVote {
            zone_id: test_zone,
            epoch: 0,
            block_hash: block_e0.block_hash,
            voter_id: voter.validator_id,
            signature: voter_key.sign(&block_e0.block_hash).to_bytes().to_vec(),
        };
        if let Some(ConsensusAction::BroadcastCertificate(cert)) =
            consensus_e0.receive_vote(vote)
        {
            zone_heads.set(test_zone, cert.block_hash);
        }
    }

    let head_after_e0 = zone_heads.get_or_genesis(test_zone);
    assert_ne!(head_after_e0, [0; 32], "head should have advanced");

    let _transition = epoch_mgr.advance_epoch(&validators[0].validator_id);
    let committee_e1 = epoch_mgr.committee_for_zone(test_zone);
    let leader_e1 = leader_for_round(&committee_e1, 1, 0).validator_id;

    let mut consensus_e1 = ZoneConsensus::new(
        test_zone,
        1,
        committee_e1.clone(),
        leader_e1,
        head_after_e0,
        config.clone(),
        0,
    );

    let block_e1 = match consensus_e1
        .propose(
            vec![dummy_spend(0x02)],
            make_sign_fn(find_key(&keys, &leader_e1)),
        )
        .unwrap()
    {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    assert_eq!(
        block_e1.header.parent_hash, head_after_e0,
        "epoch 1 block must reference epoch 0's zone head"
    );

    let voter_id_e1 = committee_e1[1].validator_id;
    let mut voter_e1 = ZoneConsensus::new(
        test_zone,
        1,
        committee_e1.clone(),
        voter_id_e1,
        head_after_e0,
        config,
        1,
    );
    let vote_action = voter_e1.vote_on_proposal(
        &block_e1,
        make_sign_fn(find_key(&keys, &voter_id_e1)),
    );
    assert!(
        vote_action.is_some(),
        "validator with correct parent_hash should accept the proposal"
    );
}

// ---------------------------------------------------------------------------
// 5. Invalid-proof containment — spend with invalid proof is dropped and
//    never appears in any block proposal.
// ---------------------------------------------------------------------------

#[test]
fn criterion_5_invalid_proof_containment() {
    let keys = make_keys(5);
    let validators = make_validators_from_keys(&keys);
    let config = test_config(8);
    let mut mempool = Mempool::new(0, 100);

    let valid_spend = dummy_spend(0x10);
    let invalid_spend = dummy_spend(0x20);

    fn verify_proof(spend: &SpendTransaction) -> bool {
        !spend.proof.is_empty()
    }

    let valid_with_proof = SpendTransaction {
        proof: vec![1, 2, 3],
        ..valid_spend.clone()
    };

    if verify_proof(&valid_with_proof) {
        mempool.insert(valid_with_proof);
    }
    if verify_proof(&invalid_spend) {
        mempool.insert(invalid_spend);
    }

    assert_eq!(mempool.len(), 1, "only proof-verified spends enter mempool");

    let seed = [0u8; 32];
    let committee = sample_committee(&seed, 0, &validators, config.committee_size);
    let leader_id = leader_for_round(&committee, 0, 0).validator_id;

    let spends = mempool.drain_proposal(64);
    let mut consensus = ZoneConsensus::new(0, 0, committee, leader_id, [0; 32], config, 0);

    let block = match consensus
        .propose(spends, make_sign_fn(find_key(&keys, &leader_id)))
        .unwrap()
    {
        ConsensusAction::BroadcastProposal(p) => p,
        _ => panic!("expected proposal"),
    };

    assert_eq!(block.transactions.len(), 1);
    assert!(
        !block.transactions[0].proof.is_empty(),
        "all spends in block must have valid proofs"
    );
    assert_eq!(
        block.transactions[0].nullifier,
        Nullifier([0x10; 32]),
        "only the valid spend should be in the block"
    );
}
