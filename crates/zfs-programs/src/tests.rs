use crate::program_topic;
use crate::zchat::{sector_id_for_channel, ChannelId, ZChatDescriptor, ZChatMessage};
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
    };
    assert_ne!(d1.program_id().expect("id1"), d2.program_id().expect("id2"));
}

#[test]
fn zid_message_round_trip() {
    let msg = ZidMessage {
        owner_did: "did:key:z6Mkt...".into(),
        display_name: Some("Alice".into()),
        timestamp_ms: 1700000000000,
    };
    let bytes = msg.encode_canonical().expect("encode");
    let decoded = ZidMessage::decode_canonical(&bytes).expect("decode");
    assert_eq!(msg, decoded);
}

#[test]
fn zchat_program_id_deterministic() {
    let d1 = ZChatDescriptor::v1();
    let d2 = ZChatDescriptor::v1();
    assert_eq!(d1.program_id().expect("id1"), d2.program_id().expect("id2"));
}

#[test]
fn zchat_descriptor_round_trip() {
    let original = ZChatDescriptor::v1();
    let bytes = original.encode_canonical().expect("encode");
    let decoded = ZChatDescriptor::decode_canonical(&bytes).expect("decode");
    assert_eq!(original, decoded);
}

#[test]
fn zid_and_zchat_have_different_program_ids() {
    let zid_id = ZidDescriptor::v1().program_id().expect("zid");
    let zchat_id = ZChatDescriptor::v1().program_id().expect("zchat");
    assert_ne!(zid_id, zchat_id);
}

#[test]
fn topic_format() {
    let desc = ZidDescriptor::v1();
    let topic = desc.topic().expect("topic");
    assert!(topic.starts_with("prog/"));
    assert_eq!(topic.len(), 5 + 64); // "prog/" + 64 hex chars
}

#[test]
fn program_topic_matches_descriptor_topic() {
    let desc = ZChatDescriptor::v1();
    let pid = desc.program_id().expect("pid");
    assert_eq!(program_topic(&pid), desc.topic().expect("topic"));
}

#[test]
fn channel_id_sector_deterministic() {
    let ch1 = ChannelId::from_str_id("general");
    let ch2 = ChannelId::from_str_id("general");
    assert_eq!(ch1.sector_id(), ch2.sector_id());
}

#[test]
fn different_channels_different_sectors() {
    let ch1 = ChannelId::from_str_id("general");
    let ch2 = ChannelId::from_str_id("random");
    assert_ne!(ch1.sector_id(), ch2.sector_id());
}

#[test]
fn sector_id_for_channel_consistency() {
    let ch = ChannelId::from_str_id("test-channel");
    let sid1 = sector_id_for_channel(&ch);
    let sid2 = ch.sector_id();
    assert_eq!(sid1.as_bytes(), sid2.as_bytes());
}

#[test]
fn zchat_message_round_trip() {
    let msg = ZChatMessage {
        sender_did: "did:key:z6Mk...".into(),
        channel_id: ChannelId::from_str_id("general"),
        content: "Hello, world!".into(),
        timestamp_ms: 1700000000000,
    };
    let bytes = msg.encode_canonical().expect("encode");
    let decoded = ZChatMessage::decode_canonical(&bytes).expect("decode");
    assert_eq!(msg, decoded);
}

#[test]
fn zchat_message_size_limit() {
    let large_content = "x".repeat(100_000);
    let msg = ZChatMessage {
        sender_did: "did:key:z6Mk...".into(),
        channel_id: ChannelId::from_str_id("big"),
        content: large_content,
        timestamp_ms: 0,
    };
    let result = msg.encode_canonical();
    assert!(result.is_err());
}

#[test]
fn test_channel_id_reserved() {
    let ch = ChannelId::from_str_id(crate::zchat::TEST_CHANNEL_ID);
    assert_eq!(ch.as_bytes(), b"INTERLINK-MAIN");
    let _ = ch.sector_id();
}

#[test]
fn zchat_message_empty_content() {
    let msg = ZChatMessage {
        sender_did: "did:key:z6Mk...".into(),
        channel_id: ChannelId::new(vec![1, 2, 3]),
        content: String::new(),
        timestamp_ms: 42,
    };
    let bytes = msg.encode_canonical().expect("encode");
    let decoded = ZChatMessage::decode_canonical(&bytes).expect("decode");
    assert_eq!(msg, decoded);
}
