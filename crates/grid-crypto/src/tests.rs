use crate::{decrypt_sector, encrypt_sector, unwrap_sector_key, wrap_sector_key, SectorKey};
use grid_core::{ProgramDescriptor, ProgramId, SectorId};
use zid::testkit::derive_machine_keypair_from_seed;
use zid::MachineKeyCapabilities;

fn test_aad(program_id: &ProgramId, sector_id: &SectorId) -> Vec<u8> {
    let mut aad = Vec::new();
    aad.extend_from_slice(program_id.as_bytes());
    aad.extend_from_slice(sector_id.as_bytes());
    aad
}

fn make_keypair(seed: u8) -> zid::MachineKeyPair {
    let mut nk_bytes = [0u8; 32];
    nk_bytes[0] = seed;
    derive_machine_keypair_from_seed(
        nk_bytes,
        zid::IdentityId::new([seed; 16]),
        zid::MachineId::new([seed; 16]),
        0,
        MachineKeyCapabilities::all(),
    )
    .expect("keypair derivation")
}

#[test]
fn encrypt_decrypt_round_trip() {
    let key = SectorKey::generate();
    let program_id = ProgramId::from([1u8; 32]);
    let sector_id = SectorId::from_bytes(vec![2u8; 16]);
    let aad = test_aad(&program_id, &sector_id);

    let plaintext = b"hello, encrypted world!";
    let sealed = encrypt_sector(plaintext, &key, &aad).expect("encrypt");
    let recovered = decrypt_sector(&sealed, &key, &aad).expect("decrypt");
    assert_eq!(recovered, plaintext);
}

#[test]
fn encrypt_decrypt_empty_plaintext() {
    let key = SectorKey::generate();
    let aad = b"context";
    let sealed = encrypt_sector(b"", &key, aad).expect("encrypt empty");
    let recovered = decrypt_sector(&sealed, &key, aad).expect("decrypt empty");
    assert!(recovered.is_empty());
}

#[test]
fn wrong_key_rejected() {
    let key1 = SectorKey::generate();
    let key2 = SectorKey::generate();
    let aad = b"context";

    let sealed = encrypt_sector(b"secret data", &key1, aad).expect("encrypt");
    let result = decrypt_sector(&sealed, &key2, aad);
    assert!(result.is_err());
}

#[test]
fn aad_mismatch_rejected() {
    let key = SectorKey::generate();
    let sealed = encrypt_sector(b"data", &key, b"aad1").expect("encrypt");
    let result = decrypt_sector(&sealed, &key, b"aad2");
    assert!(result.is_err());
}

#[test]
fn ciphertext_too_short() {
    let key = SectorKey::generate();
    let result = decrypt_sector(&[0u8; 10], &key, b"aad");
    assert!(result.is_err());
}

#[test]
fn wrap_unwrap_round_trip() {
    let sender_kp = make_keypair(1);
    let recipient_kp = make_keypair(2);

    let sector_key = SectorKey::generate();
    let program_id = ProgramId::from([0xAA; 32]);
    let sector_id = SectorId::from_bytes(vec![0xBB; 16]);

    let entry = wrap_sector_key(
        &sector_key,
        &sender_kp,
        &recipient_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("wrap");

    assert_eq!(entry.wrapped_key.0.len(), 24 + 32 + 16); // nonce + key + tag

    let recovered = unwrap_sector_key(
        &entry,
        &recipient_kp,
        &sender_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("unwrap");

    assert_eq!(recovered.as_bytes(), sector_key.as_bytes());
}

#[test]
fn unwrap_wrong_recipient_fails() {
    let sender_kp = make_keypair(1);
    let recipient_kp = make_keypair(2);
    let wrong_kp = make_keypair(3);

    let sector_key = SectorKey::generate();
    let program_id = ProgramId::from([0xCC; 32]);
    let sector_id = SectorId::from_bytes(vec![0xDD; 16]);

    let entry = wrap_sector_key(
        &sector_key,
        &sender_kp,
        &recipient_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("wrap");

    let result = unwrap_sector_key(
        &entry,
        &wrong_kp,
        &sender_kp.public_key(),
        &program_id,
        &sector_id,
    );
    assert!(result.is_err());
}

#[test]
fn unwrap_wrong_context_fails() {
    let sender_kp = make_keypair(1);
    let recipient_kp = make_keypair(2);

    let sector_key = SectorKey::generate();
    let program_id = ProgramId::from([0xEE; 32]);
    let sector_id = SectorId::from_bytes(vec![0xFF; 16]);
    let wrong_sector_id = SectorId::from_bytes(vec![0x00; 16]);

    let entry = wrap_sector_key(
        &sector_key,
        &sender_kp,
        &recipient_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("wrap");

    let result = unwrap_sector_key(
        &entry,
        &recipient_kp,
        &sender_kp.public_key(),
        &program_id,
        &wrong_sector_id,
    );
    assert!(result.is_err());
}

#[test]
fn key_envelope_entry_serialization() {
    let sender_kp = make_keypair(10);
    let recipient_kp = make_keypair(20);

    let sector_key = SectorKey::generate();
    let program_id = ProgramId::from([0x11; 32]);
    let sector_id = SectorId::from_bytes(vec![0x22; 8]);

    let entry = wrap_sector_key(
        &sector_key,
        &sender_kp,
        &recipient_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("wrap");

    let encoded = grid_core::encode_canonical(&entry).expect("encode");
    let decoded: crate::KeyEnvelopeEntry = grid_core::decode_canonical(&encoded).expect("decode");

    assert_eq!(decoded.recipient_did, entry.recipient_did);
    assert_eq!(decoded.sender_x25519_public, entry.sender_x25519_public);
    assert_eq!(decoded.mlkem_ciphertext, entry.mlkem_ciphertext);
    assert_eq!(decoded.wrapped_key, entry.wrapped_key);
}

#[test]
fn full_encrypt_wrap_unwrap_decrypt() {
    let sender_kp = make_keypair(5);
    let recipient_kp = make_keypair(6);

    let descriptor = ProgramDescriptor {
        name: "test-program".into(),
        version: "1".into(),
    };
    let program_id = descriptor.program_id().expect("program_id");
    let sector_id = SectorId::from_bytes(b"test-sector".to_vec());
    let aad = test_aad(&program_id, &sector_id);

    let sector_key = SectorKey::generate();
    let plaintext = b"end-to-end encrypted content";

    let sealed = encrypt_sector(plaintext, &sector_key, &aad).expect("encrypt");

    let entry = wrap_sector_key(
        &sector_key,
        &sender_kp,
        &recipient_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("wrap");

    let recovered_key = unwrap_sector_key(
        &entry,
        &recipient_kp,
        &sender_kp.public_key(),
        &program_id,
        &sector_id,
    )
    .expect("unwrap");

    let recovered = decrypt_sector(&sealed, &recovered_key, &aad).expect("decrypt");
    assert_eq!(recovered, plaintext);
}

#[test]
fn sector_key_debug_redacts() {
    let key = SectorKey::generate();
    let debug = format!("{:?}", key);
    assert_eq!(debug, "SectorKey([REDACTED])");
}

// ---------------------------------------------------------------------------
// Poseidon sponge encryption
// ---------------------------------------------------------------------------

use crate::{
    poseidon_decrypt, poseidon_decrypt_sector, poseidon_encrypt, poseidon_encrypt_sector,
    poseidon_hash,
};

#[test]
fn poseidon_encrypt_decrypt_round_trip() {
    let key = SectorKey::generate();
    let nonce = [0u8; 32];
    let aad = b"test-aad";
    let plaintext = b"hello poseidon world";

    let sealed = poseidon_encrypt(plaintext, &key, &nonce, aad).expect("encrypt");
    let recovered = poseidon_decrypt(&sealed, &key, &nonce, aad).expect("decrypt");
    assert_eq!(recovered, plaintext);
}

#[test]
fn poseidon_encrypt_decrypt_empty() {
    let key = SectorKey::generate();
    let nonce = [0u8; 32];
    let aad = b"test-aad";

    let sealed = poseidon_encrypt(b"", &key, &nonce, aad).expect("encrypt empty");
    let recovered = poseidon_decrypt(&sealed, &key, &nonce, aad).expect("decrypt empty");
    assert!(recovered.is_empty());
}

#[test]
fn poseidon_wrong_key_rejected() {
    let key1 = SectorKey::generate();
    let key2 = SectorKey::generate();
    let nonce = [0u8; 32];
    let aad = b"test-aad";

    let sealed = poseidon_encrypt(b"secret data", &key1, &nonce, aad).expect("encrypt");
    let result = poseidon_decrypt(&sealed, &key2, &nonce, aad);
    assert!(result.is_err());
}

#[test]
fn poseidon_aad_mismatch_rejected() {
    let key = SectorKey::generate();
    let nonce = [0u8; 32];

    let sealed = poseidon_encrypt(b"data", &key, &nonce, b"aad1").expect("encrypt");
    let result = poseidon_decrypt(&sealed, &key, &nonce, b"aad2");
    assert!(result.is_err());
}

#[test]
fn poseidon_sector_round_trip() {
    let key = SectorKey::generate();
    let aad = b"program||sector";
    let plaintext = b"sector payload for poseidon";

    let sealed = poseidon_encrypt_sector(plaintext, &key, aad).expect("encrypt_sector");
    let recovered = poseidon_decrypt_sector(&sealed, &key, aad).expect("decrypt_sector");
    assert_eq!(recovered, plaintext);
}

#[test]
fn poseidon_hash_deterministic() {
    let data = b"deterministic hash input";
    let h1 = poseidon_hash(data);
    let h2 = poseidon_hash(data);
    assert_eq!(h1, h2);
}

#[test]
fn poseidon_hash_different_data() {
    let h1 = poseidon_hash(b"input A");
    let h2 = poseidon_hash(b"input B");
    assert_ne!(h1, h2);
}

// ---------------------------------------------------------------------------
// Padding (pad_to_bucket / unpad_from_bucket)
// ---------------------------------------------------------------------------

use crate::{pad_to_bucket, unpad_from_bucket};

#[test]
fn pad_unpad_round_trip() {
    let data = b"hello, padded world!";
    let padded = pad_to_bucket(data);
    let recovered = unpad_from_bucket(&padded).expect("unpad");
    assert_eq!(recovered, data);
}

#[test]
fn pad_unpad_empty_data() {
    let padded = pad_to_bucket(b"");
    assert_eq!(
        padded.len(),
        256,
        "empty content should pad to smallest bucket"
    );
    let recovered = unpad_from_bucket(&padded).expect("unpad empty");
    assert!(recovered.is_empty());
}

#[test]
fn padded_size_is_bucket_aligned() {
    let bucket_sizes: &[usize] = &[
        256, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072, 262_144,
    ];
    for &size in bucket_sizes {
        if size < 4 {
            continue;
        }
        let content_len = size - 4; // exactly fills the bucket (4-byte header + content)
        let content = vec![0xABu8; content_len];
        let padded = pad_to_bucket(&content);
        assert_eq!(
            padded.len(),
            size,
            "content of len {content_len} should fit exactly in bucket {size}"
        );
        let recovered = unpad_from_bucket(&padded).expect("unpad");
        assert_eq!(recovered, content);
    }
}

#[test]
fn content_just_over_bucket_goes_to_next() {
    let content = vec![0xCDu8; 253]; // 4 + 253 = 257, exceeds 256 bucket → 512
    let padded = pad_to_bucket(&content);
    assert_eq!(padded.len(), 512);
    let recovered = unpad_from_bucket(&padded).expect("unpad");
    assert_eq!(recovered, content);
}

#[test]
fn oversized_content_rounds_to_largest_bucket_multiple() {
    let largest_bucket = 262_144;
    let content_len = largest_bucket; // 4 + 262144 > 262144 → needs 2 × 262144
    let content = vec![0xFFu8; content_len];
    let padded = pad_to_bucket(&content);
    assert_eq!(padded.len(), 2 * largest_bucket);
    let recovered = unpad_from_bucket(&padded).expect("unpad");
    assert_eq!(recovered, content);
}

#[test]
fn unpad_too_short_fails() {
    let result = unpad_from_bucket(&[0u8; 3]);
    assert!(result.is_err());
}

#[test]
fn unpad_declared_length_exceeds_buffer_fails() {
    let mut bad = vec![0u8; 256];
    let fake_len: u32 = 300;
    bad[..4].copy_from_slice(&fake_len.to_le_bytes());
    let result = unpad_from_bucket(&bad);
    assert!(result.is_err());
}

#[test]
fn pad_stores_length_as_le_u32() {
    let data = vec![0x42u8; 100];
    let padded = pad_to_bucket(&data);
    let stored_len = u32::from_le_bytes([padded[0], padded[1], padded[2], padded[3]]);
    assert_eq!(stored_len, 100);
}

#[test]
fn padding_bytes_are_zero() {
    let data = b"short";
    let padded = pad_to_bucket(data);
    let payload_end = 4 + data.len();
    for &b in &padded[payload_end..] {
        assert_eq!(b, 0, "padding bytes must be zero");
    }
}

#[test]
fn various_sizes_round_trip() {
    for size in [1, 10, 100, 252, 508, 1020, 4092, 50_000, 200_000] {
        let content = vec![(size & 0xFF) as u8; size];
        let padded = pad_to_bucket(&content);
        let recovered = unpad_from_bucket(&padded).expect("unpad");
        assert_eq!(
            recovered, content,
            "round-trip failed for content size {size}"
        );
    }
}
