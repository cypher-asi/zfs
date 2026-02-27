use crate::{decrypt_sector, encrypt_sector, unwrap_sector_key, wrap_sector_key, SectorKey};
use zid::testkit::derive_machine_keypair_from_seed;
use zid::MachineKeyCapabilities;
use grid_core::{ProgramDescriptor, ProgramId, SectorId};

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

    assert_eq!(entry.wrapped_key.len(), 24 + 32 + 16); // nonce + key + tag

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
