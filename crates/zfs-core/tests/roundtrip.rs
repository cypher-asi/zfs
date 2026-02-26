use zfs_core::*;

// --- Cid ---

#[test]
fn cid_from_ciphertext_is_deterministic() {
    let data = b"hello world ciphertext";
    let cid1 = Cid::from_ciphertext(data);
    let cid2 = Cid::from_ciphertext(data);
    assert_eq!(cid1, cid2);
}

#[test]
fn cid_different_data_different_ids() {
    let a = Cid::from_ciphertext(b"alpha");
    let b = Cid::from_ciphertext(b"beta");
    assert_ne!(a, b);
}

#[test]
fn cid_hex_roundtrip() {
    let cid = Cid::from_ciphertext(b"test data for hex");
    let hex = cid.to_hex();
    assert_eq!(hex.len(), 64);
    let parsed = Cid::from_hex(&hex).unwrap();
    assert_eq!(cid, parsed);
}

#[test]
fn cid_from_bytes() {
    let bytes = [0xAB; 32];
    let cid = Cid::from(bytes);
    assert_eq!(*cid.as_bytes(), bytes);
}

#[test]
fn cid_cbor_roundtrip() {
    let cid = Cid::from_ciphertext(b"cbor test");
    let encoded = encode_canonical(&cid).unwrap();
    let decoded: Cid = decode_canonical(&encoded).unwrap();
    assert_eq!(cid, decoded);
}

#[test]
fn cid_display_is_hex() {
    let cid = Cid::from([0x00; 32]);
    assert_eq!(format!("{cid}"), "0".repeat(64));
}

// --- SectorId ---

#[test]
fn sector_id_cbor_roundtrip() {
    let sid = SectorId::from_bytes(vec![1, 2, 3, 4, 5]);
    let encoded = encode_canonical(&sid).unwrap();
    let decoded: SectorId = decode_canonical(&encoded).unwrap();
    assert_eq!(sid, decoded);
}

#[test]
fn sector_id_empty_roundtrip() {
    let sid = SectorId::from_bytes(vec![]);
    let encoded = encode_canonical(&sid).unwrap();
    let decoded: SectorId = decode_canonical(&encoded).unwrap();
    assert_eq!(sid, decoded);
}

#[test]
fn sector_id_hex() {
    let sid = SectorId::from_bytes(vec![0xDE, 0xAD]);
    assert_eq!(sid.to_hex(), "dead");
}

// --- ProgramId ---

#[test]
fn program_id_from_descriptor_deterministic() {
    let desc = ProgramDescriptor {
        name: "test-program".into(),
        version: "1.0.0".into(),
    };
    let id1 = desc.program_id().unwrap();
    let id2 = desc.program_id().unwrap();
    assert_eq!(id1, id2);
}

#[test]
fn program_id_different_descriptors_different_ids() {
    let d1 = ProgramDescriptor {
        name: "alpha".into(),
        version: "1.0".into(),
    };
    let d2 = ProgramDescriptor {
        name: "beta".into(),
        version: "1.0".into(),
    };
    assert_ne!(d1.program_id().unwrap(), d2.program_id().unwrap());
}

#[test]
fn program_id_hex_roundtrip() {
    let desc = ProgramDescriptor {
        name: "hex-test".into(),
        version: "0.1".into(),
    };
    let id = desc.program_id().unwrap();
    let hex = id.to_hex();
    assert_eq!(hex.len(), 64);
    let parsed = ProgramId::from_hex(&hex).unwrap();
    assert_eq!(id, parsed);
}

#[test]
fn program_id_cbor_roundtrip() {
    let id = ProgramId::from([0xFF; 32]);
    let encoded = encode_canonical(&id).unwrap();
    let decoded: ProgramId = decode_canonical(&encoded).unwrap();
    assert_eq!(id, decoded);
}

// --- ProgramDescriptor ---

#[test]
fn program_descriptor_cbor_roundtrip() {
    let desc = ProgramDescriptor {
        name: "my-program".into(),
        version: "0.1.0".into(),
    };
    let encoded = desc.encode_canonical().unwrap();
    let decoded = ProgramDescriptor::decode_canonical(&encoded).unwrap();
    assert_eq!(desc, decoded);
}

#[test]
fn program_descriptor_canonical_encoding_is_deterministic() {
    let desc = ProgramDescriptor {
        name: "stable".into(),
        version: "2.0".into(),
    };
    let enc1 = desc.encode_canonical().unwrap();
    let enc2 = desc.encode_canonical().unwrap();
    assert_eq!(enc1, enc2);
}

// --- ErrorCode / ZfsError ---

#[test]
fn error_code_to_zfs_error_roundtrip() {
    let codes = [
        ErrorCode::StorageFull,
        ErrorCode::ProofInvalid,
        ErrorCode::PolicyReject,
        ErrorCode::NotFound,
        ErrorCode::InvalidPayload,
        ErrorCode::ProgramMismatch,
    ];
    for code in codes {
        let err: ZfsError = code.into();
        assert_eq!(err.error_code(), Some(code));
    }
}

#[test]
fn error_code_cbor_roundtrip() {
    let code = ErrorCode::ProofInvalid;
    let encoded = encode_canonical(&code).unwrap();
    let decoded: ErrorCode = decode_canonical(&encoded).unwrap();
    assert_eq!(code, decoded);
}

#[test]
fn zfs_error_io_has_no_code() {
    let err = ZfsError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
    assert_eq!(err.error_code(), None);
}

// --- Sector protocol messages (append model) ---

#[test]
fn sector_append_request_roundtrip() {
    let req = SectorAppendRequest {
        program_id: ProgramId::from([0x11; 32]),
        sector_id: SectorId::from_bytes(vec![0xAA; 32]),
        entry: vec![0xAB; 64],
        shape_proof: None,
    };
    let encoded = encode_canonical(&req).unwrap();
    let decoded: SectorAppendRequest = decode_canonical(&encoded).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn sector_log_length_response_roundtrip() {
    let resp = SectorLogLengthResponse {
        length: 42,
        error_code: None,
    };
    let encoded = encode_canonical(&resp).unwrap();
    let decoded: SectorLogLengthResponse = decode_canonical(&encoded).unwrap();
    assert_eq!(resp, decoded);
}

#[test]
fn sector_request_enum_roundtrip() {
    let req = SectorRequest::LogLength(SectorLogLengthRequest {
        program_id: ProgramId::from([0x22; 32]),
        sector_id: SectorId::from_bytes(vec![0xBB; 32]),
    });
    let encoded = encode_canonical(&req).unwrap();
    let decoded: SectorRequest = decode_canonical(&encoded).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn gossip_sector_append_roundtrip() {
    let gs = GossipSectorAppend {
        program_id: ProgramId::from([0x33; 32]),
        sector_id: SectorId::from_bytes(vec![0xCC; 32]),
        index: 7,
        payload: vec![0xEF; 128],
        shape_proof: None,
    };
    let encoded = encode_canonical(&gs).unwrap();
    let decoded: GossipSectorAppend = decode_canonical(&encoded).unwrap();
    assert_eq!(gs, decoded);
}
