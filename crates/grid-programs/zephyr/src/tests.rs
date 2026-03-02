use crate::*;

#[test]
fn zone_descriptor_program_id_is_deterministic() {
    let d1 = ZephyrZoneDescriptor::new(42);
    let d2 = ZephyrZoneDescriptor::new(42);
    assert_eq!(d1.program_id().unwrap(), d2.program_id().unwrap());
}

#[test]
fn different_zones_produce_different_program_ids() {
    let d1 = ZephyrZoneDescriptor::new(0);
    let d2 = ZephyrZoneDescriptor::new(1);
    assert_ne!(d1.program_id().unwrap(), d2.program_id().unwrap());
}

#[test]
fn global_descriptor_program_id_is_deterministic() {
    let d1 = ZephyrGlobalDescriptor::new();
    let d2 = ZephyrGlobalDescriptor::new();
    assert_eq!(d1.program_id().unwrap(), d2.program_id().unwrap());
}

#[test]
fn spend_descriptor_defaults_to_groth16() {
    let d = ZephyrSpendDescriptor::new();
    assert_eq!(d.proof_system, grid_core::ProofSystem::Groth16);
}

#[test]
fn descriptor_cbor_round_trip() {
    let original = ZephyrZoneDescriptor::new(99);
    let bytes = original.encode_canonical().unwrap();
    let decoded = ZephyrZoneDescriptor::decode_canonical(&bytes).unwrap();
    assert_eq!(original, decoded);
}

#[test]
fn global_descriptor_cbor_round_trip() {
    let original = ZephyrGlobalDescriptor::new();
    let bytes = original.encode_canonical().unwrap();
    let decoded = ZephyrGlobalDescriptor::decode_canonical(&bytes).unwrap();
    assert_eq!(original, decoded);
}

#[test]
fn spend_descriptor_cbor_round_trip() {
    let original = ZephyrSpendDescriptor::new();
    let bytes = original.encode_canonical().unwrap();
    let decoded = ZephyrSpendDescriptor::decode_canonical(&bytes).unwrap();
    assert_eq!(original, decoded);
}

#[test]
fn validator_descriptor_cbor_round_trip() {
    let original = ZephyrValidatorDescriptor::new();
    let bytes = original.encode_canonical().unwrap();
    let decoded = ZephyrValidatorDescriptor::decode_canonical(&bytes).unwrap();
    assert_eq!(original, decoded);
}

#[test]
fn zone_descriptor_topic_format() {
    let d = ZephyrZoneDescriptor::new(0);
    let topic = d.topic().unwrap();
    assert!(topic.starts_with("prog/"));
    assert_eq!(topic.len(), 5 + 64);
}

#[test]
fn types_cbor_round_trip() {
    let nullifier = Nullifier([0xAB; 32]);
    let bytes = grid_core::encode_canonical(&nullifier).unwrap();
    let decoded: Nullifier = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(nullifier, decoded);

    let commitment = NoteCommitment([0xCD; 32]);
    let bytes = grid_core::encode_canonical(&commitment).unwrap();
    let decoded: NoteCommitment = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(commitment, decoded);
}

#[test]
fn spend_transaction_cbor_round_trip() {
    let tx = SpendTransaction {
        input_commitment: NoteCommitment([1; 32]),
        nullifier: Nullifier([2; 32]),
        outputs: vec![NoteOutput {
            commitment: NoteCommitment([3; 32]),
            encrypted_data: vec![4, 5, 6],
        }],
        proof: vec![7, 8, 9],
        public_signals: vec![[10; 32]],
    };
    let bytes = grid_core::encode_canonical(&tx).unwrap();
    let decoded: SpendTransaction = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(tx, decoded);
}

#[test]
fn block_cbor_round_trip() {
    let block = Block {
        header: BlockHeader {
            zone_id: 42,
            epoch: 1,
            height: 0,
            parent_hash: [0; 32],
            transactions_root: [0xAA; 32],
            timestamp_ms: 1_700_000_000_000,
            proposer_id: [3; 32],
        },
        transactions: vec![],
        block_hash: [2; 32],
        proposer_sig: vec![4, 5],
    };
    let bytes = grid_core::encode_canonical(&block).unwrap();
    let decoded: Block = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(block, decoded);
}

#[test]
fn finality_certificate_cbor_round_trip() {
    let cert = FinalityCertificate {
        zone_id: 10,
        epoch: 5,
        parent_hash: [0xAA; 32],
        block_hash: [0xCC; 32],
        signatures: vec![CertSignature {
            validator_id: [0xDD; 32],
            signature: vec![0xEE, 0xFF],
        }],
    };
    let bytes = grid_core::encode_canonical(&cert).unwrap();
    let decoded: FinalityCertificate = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(cert, decoded);
}

#[test]
fn zone_message_cbor_round_trip() {
    let msg = ZephyrZoneMessage::Reject(SpendReject {
        nullifier: Nullifier([0x11; 32]),
        reason: RejectReason::DuplicateNullifier,
    });
    let bytes = grid_core::encode_canonical(&msg).unwrap();
    let decoded: ZephyrZoneMessage = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn global_message_cbor_round_trip() {
    let msg = ZephyrGlobalMessage::EpochAnnounce(EpochAnnouncement {
        epoch: 42,
        randomness_seed: [0xFF; 32],
        start_time_ms: 1_700_000_000_000,
    });
    let bytes = grid_core::encode_canonical(&msg).unwrap();
    let decoded: ZephyrGlobalMessage = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn validator_info_cbor_round_trip() {
    let info = ValidatorInfo {
        validator_id: [1; 32],
        pubkey: [2; 32],
        p2p_endpoint: "/ip4/127.0.0.1/tcp/4001".to_owned(),
    };
    let bytes = grid_core::encode_canonical(&info).unwrap();
    let decoded: ValidatorInfo = grid_core::decode_canonical(&bytes).unwrap();
    assert_eq!(info, decoded);
}
