use crate::zid::{ZidDescriptor, ZidMessage};

#[test]
fn zid_program_id_deterministic() {
    let d1 = ZidDescriptor::v1();
    let d2 = ZidDescriptor::v1();
    assert_eq!(d1.program_id().expect("id1"), d2.program_id().expect("id2"));
}

#[test]
fn zid_descriptor_round_trip() {
    let original = ZidDescriptor::v1();
    let bytes = original.encode_canonical().expect("encode");
    let decoded = ZidDescriptor::decode_canonical(&bytes).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn zid_different_versions_different_ids() {
    let d1 = ZidDescriptor::v1();
    let d2 = ZidDescriptor {
        name: "zid".into(),
        version: 2,
        proof_required: false,
        proof_system: None,
    };
    assert_ne!(d1.program_id().expect("id1"), d2.program_id().expect("id2"));
}

#[test]
fn zid_message_round_trip() {
    let msg = ZidMessage {
        owner_did: "did:key:z6Mkt...".into(),
        display_name: Some("Alice".into()),
        timestamp_ms: 1700000000000,
        signature: vec![],
    };
    let bytes = msg.encode_canonical().expect("encode");
    let decoded = ZidMessage::decode_canonical(&bytes).expect("decode");
    assert_eq!(msg, decoded);
}

#[test]
fn topic_format() {
    let desc = ZidDescriptor::v1();
    let topic = desc.topic().expect("topic");
    assert!(topic.starts_with("prog/"));
    assert_eq!(topic.len(), 5 + 64);
}

#[test]
fn zid_message_with_signature_round_trip() {
    let msg = ZidMessage {
        owner_did: "did:key:z6Mkt...".into(),
        display_name: Some("Alice".into()),
        timestamp_ms: 1700000000000,
        signature: vec![0xCA, 0xFE],
    };
    let bytes = msg.encode_canonical().expect("encode");
    let decoded = ZidMessage::decode_canonical(&bytes).expect("decode");
    assert_eq!(msg, decoded);
}

#[test]
fn zid_signable_bytes_deterministic() {
    let msg = ZidMessage {
        owner_did: "did:key:z6Mkt...".into(),
        display_name: Some("Bob".into()),
        timestamp_ms: 1700000000000,
        signature: vec![1, 2, 3],
    };
    let sb1 = msg.signable_bytes().expect("signable_bytes 1");
    let sb2 = msg.signable_bytes().expect("signable_bytes 2");
    assert_eq!(sb1, sb2);
}

#[test]
fn zid_field_schema_has_signature_field() {
    let schema = ZidDescriptor::field_schema();
    let has_sig = schema.fields.iter().any(|f| f.key == "signature");
    assert!(has_sig, "ZID field schema must contain a 'signature' field");
}
