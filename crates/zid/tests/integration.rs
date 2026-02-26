use zid::testkit::{
    derive_identity_signing_key_from_seed, derive_machine_keypair_from_seed,
};
use zid::*;

fn test_identity() -> ([u8; 16], IdentitySigningKey) {
    let seed: [u8; 32] = rand::random();
    let identity_id: [u8; 16] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];
    let isk = derive_identity_signing_key_from_seed(seed, &identity_id).unwrap();
    (identity_id, isk)
}

fn test_machine_ids() -> ([u8; 16], [u8; 16]) {
    let identity_id: [u8; 16] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ];
    let machine_id: [u8; 16] = [
        0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF,
        0xB0,
    ];
    (identity_id, machine_id)
}

// --- Deterministic derivation ---

#[test]
fn deterministic_isk_derivation() {
    let seed = [0x42u8; 32];
    let identity_id = [0x01u8; 16];

    let isk1 = derive_identity_signing_key_from_seed(seed, &identity_id).unwrap();
    let isk2 = derive_identity_signing_key_from_seed(seed, &identity_id).unwrap();

    assert_eq!(
        isk1.ed25519_public_bytes(),
        isk2.ed25519_public_bytes(),
        "same seed + identity_id must produce identical Ed25519 keys"
    );
}

#[test]
fn deterministic_machine_key_derivation() {
    let seed = [0x77u8; 32];
    let (identity_id, machine_id) = test_machine_ids();
    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;

    let mk1 = derive_machine_keypair_from_seed(seed, &identity_id, &machine_id, 1, caps).unwrap();
    let mk2 = derive_machine_keypair_from_seed(seed, &identity_id, &machine_id, 1, caps).unwrap();

    assert_eq!(
        mk1.public_key().ed25519_bytes(),
        mk2.public_key().ed25519_bytes(),
        "same inputs must produce identical machine keys"
    );
}

#[test]
fn different_epochs_produce_different_keys() {
    let seed = [0x77u8; 32];
    let (identity_id, machine_id) = test_machine_ids();
    let caps = MachineKeyCapabilities::SIGN;

    let mk1 = derive_machine_keypair_from_seed(seed, &identity_id, &machine_id, 1, caps).unwrap();
    let mk2 = derive_machine_keypair_from_seed(seed, &identity_id, &machine_id, 2, caps).unwrap();

    assert_ne!(
        mk1.public_key().ed25519_bytes(),
        mk2.public_key().ed25519_bytes(),
        "different epochs must produce different keys"
    );
}

// --- Hybrid signature round-trip ---

#[test]
fn isk_hybrid_sign_verify() {
    let (_, isk) = test_identity();
    let vk = isk.verifying_key();

    let msg = b"hello, zid";
    let sig = isk.sign(msg);

    assert!(
        vk.verify(msg, &sig).is_ok(),
        "valid hybrid signature must verify"
    );
}

#[test]
fn machine_key_hybrid_sign_verify() {
    let seed: [u8; 32] = rand::random();
    let (identity_id, machine_id) = test_machine_ids();
    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;

    let mk = derive_machine_keypair_from_seed(seed, &identity_id, &machine_id, 1, caps).unwrap();
    let pk = mk.public_key();

    let msg = b"machine key test message";
    let sig = mk.sign(msg);

    assert!(
        pk.verify(msg, &sig).is_ok(),
        "valid machine hybrid signature must verify"
    );
}

#[test]
fn hybrid_signature_wrong_message_fails() {
    let (_, isk) = test_identity();
    let vk = isk.verifying_key();

    let sig = isk.sign(b"correct message");
    assert!(
        vk.verify(b"wrong message", &sig).is_err(),
        "signature must not verify against wrong message"
    );
}

#[test]
fn tampered_ed25519_component_rejected() {
    let (_, isk) = test_identity();
    let vk = isk.verifying_key();

    let msg = b"test";
    let mut sig = isk.sign(msg);
    sig.ed25519[0] ^= 0xFF;

    assert!(
        vk.verify(msg, &sig).is_err(),
        "tampered Ed25519 component must cause verification failure"
    );
}

#[test]
fn tampered_ml_dsa_component_rejected() {
    let (_, isk) = test_identity();
    let vk = isk.verifying_key();

    let msg = b"test";
    let mut sig = isk.sign(msg);
    if !sig.ml_dsa.is_empty() {
        sig.ml_dsa[0] ^= 0xFF;
    }

    assert!(
        vk.verify(msg, &sig).is_err(),
        "tampered ML-DSA component must cause verification failure"
    );
}

#[test]
fn stripped_ml_dsa_component_rejected() {
    let (_, isk) = test_identity();
    let vk = isk.verifying_key();

    let msg = b"test";
    let mut sig = isk.sign(msg);
    sig.ml_dsa.clear();

    assert!(
        vk.verify(msg, &sig).is_err(),
        "stripped ML-DSA component must cause verification failure"
    );
}

// --- Hybrid signature serialization ---

#[test]
fn hybrid_signature_round_trip_bytes() {
    let (_, isk) = test_identity();

    let sig = isk.sign(b"round-trip test");
    let bytes = sig.to_bytes();
    let sig2 = HybridSignature::from_bytes(&bytes).unwrap();

    assert_eq!(sig.ed25519, sig2.ed25519);
    assert_eq!(sig.ml_dsa, sig2.ml_dsa);
}

// --- Hybrid encap/decap ---

#[test]
fn hybrid_encap_decap_round_trip() {
    let seed_a = [0xAAu8; 32];
    let seed_b = [0xBBu8; 32];
    let identity_a = [0x01u8; 16];
    let identity_b = [0x02u8; 16];
    let machine_a = [0x0Au8; 16];
    let machine_b = [0x0Bu8; 16];
    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;

    let mk_a = derive_machine_keypair_from_seed(seed_a, &identity_a, &machine_a, 1, caps).unwrap();
    let mk_b = derive_machine_keypair_from_seed(seed_b, &identity_b, &machine_b, 1, caps).unwrap();

    let pk_b = mk_b.public_key();
    let (ss_sender, bundle) = pk_b.encapsulate(&mk_a).unwrap();

    let pk_a = mk_a.public_key();
    let ss_recipient = mk_b.decapsulate(&bundle, &pk_a).unwrap();

    assert_eq!(
        ss_sender.as_bytes(),
        ss_recipient.as_bytes(),
        "sender and recipient must derive the same shared secret"
    );
}

// --- DID round-trip ---

#[test]
fn did_key_round_trip() {
    let (_, isk) = test_identity();
    let pk_bytes = isk.ed25519_public_bytes();

    let did = ed25519_to_did_key(&pk_bytes);
    assert!(
        did.starts_with("did:key:z"),
        "DID must start with did:key:z"
    );

    let recovered = did_key_to_ed25519(&did).unwrap();
    assert_eq!(
        pk_bytes, recovered,
        "DID round-trip must recover original key"
    );
}

#[test]
fn did_key_invalid_prefix_rejected() {
    assert!(did_key_to_ed25519("did:web:example.com").is_err());
}

#[test]
fn did_key_invalid_base58_rejected() {
    assert!(did_key_to_ed25519("did:key:z!!!invalid!!!").is_err());
}

// --- EncapBundle serialization ---

#[test]
fn encap_bundle_round_trip_bytes() {
    let seed_a = [0xAAu8; 32];
    let seed_b = [0xBBu8; 32];
    let identity_a = [0x01u8; 16];
    let identity_b = [0x02u8; 16];
    let machine_a = [0x0Au8; 16];
    let machine_b = [0x0Bu8; 16];
    let caps = MachineKeyCapabilities::all();

    let mk_a = derive_machine_keypair_from_seed(seed_a, &identity_a, &machine_a, 1, caps).unwrap();
    let mk_b = derive_machine_keypair_from_seed(seed_b, &identity_b, &machine_b, 1, caps).unwrap();

    let pk_b = mk_b.public_key();
    let (_, bundle) = pk_b.encapsulate(&mk_a).unwrap();

    let bytes = bundle.to_bytes();
    let bundle2 = EncapBundle::from_bytes(&bytes).unwrap();

    assert_eq!(bundle.x25519_public, bundle2.x25519_public);
    assert_eq!(bundle.mlkem_ciphertext, bundle2.mlkem_ciphertext);
}

// --- Shamir round-trip ---

#[test]
fn shamir_split_combine_round_trip() {
    let mut rng = rand::thread_rng();
    let identity_id = [0x01u8; 16];

    let bundle = generate_identity(3, 5, &identity_id, &mut rng).unwrap();
    assert_eq!(bundle.shares.len(), 5);
    assert_eq!(bundle.threshold, 3);

    let info = verify_shares(&bundle.shares[0..3], &identity_id).unwrap();
    assert_eq!(info.did, bundle.did);
}

#[test]
fn shamir_share_hex_round_trip() {
    let mut rng = rand::thread_rng();
    let identity_id = [0x02u8; 16];

    let bundle = generate_identity(2, 3, &identity_id, &mut rng).unwrap();
    for share in &bundle.shares {
        let h = share.to_hex();
        let recovered = ShamirShare::from_hex(&h).unwrap();
        assert_eq!(recovered.index(), share.index());
    }
}

#[test]
fn shares_api_sign_verify() {
    let mut rng = rand::thread_rng();
    let identity_id = [0x03u8; 16];

    let bundle = generate_identity(2, 3, &identity_id, &mut rng).unwrap();
    let msg = b"shares-based signing test";
    let sig = sign_with_shares(&bundle.shares[0..2], &identity_id, msg).unwrap();

    assert!(bundle.verifying_key.verify(msg, &sig).is_ok());
}

#[test]
fn shares_api_machine_keypair() {
    let mut rng = rand::thread_rng();
    let identity_id = [0x04u8; 16];
    let machine_id = [0x0Au8; 16];
    let caps = MachineKeyCapabilities::SIGN | MachineKeyCapabilities::ENCRYPT;

    let bundle = generate_identity(2, 3, &identity_id, &mut rng).unwrap();
    let kp = derive_machine_keypair_from_shares(
        &bundle.shares[0..2],
        &identity_id,
        &machine_id,
        1,
        caps,
    )
    .unwrap();

    let pk = kp.public_key();
    let sig = kp.sign(b"machine test");
    assert!(pk.verify(b"machine test", &sig).is_ok());
}
