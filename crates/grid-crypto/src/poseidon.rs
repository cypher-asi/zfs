use std::sync::LazyLock;

use ark_bn254::Fr;
use ark_crypto_primitives::sponge::{
    poseidon::{PoseidonConfig, PoseidonSponge},
    CryptographicSponge,
};
use ark_ff::{BigInteger, PrimeField};

use crate::{CryptoError, SectorKey};

const NONCE_LEN: usize = 32;
const TAG_LEN: usize = 32;
const RATE: usize = 2;
/// Each BN254 field element holds at most 30 usable data bytes.
/// Position 0 is a length prefix, positions 1..=30 hold data, and
/// position 31 (the LE MSB) is always zero — guaranteeing the 32-byte
/// value is below the ~2^254 BN254 scalar-field modulus and
/// `Fr::from_le_bytes_mod_order` is lossless.
const BYTES_PER_ELEMENT: usize = 30;

/// Poseidon sponge encryption.
///
/// Absorbs key, nonce, and AAD into the sponge state, then processes
/// plaintext in duplex mode: absorb plaintext element, permute, squeeze
/// ciphertext element.
///
/// Field: BN254 scalar field (Fr). Each field element holds ~31 bytes.
/// Rate: 2 field elements per permutation. Capacity: 1 field element.
///
/// Returns `nonce (32) || ciphertext_elements || tag (32)`.
pub fn poseidon_encrypt(
    plaintext: &[u8],
    key: &SectorKey,
    nonce: &[u8; 32],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mut sponge = PoseidonSponge::<Fr>::new(&POSEIDON_CONFIG);

    let key_elems = bytes_to_field_elements(key.as_bytes());
    let nonce_elems = bytes_to_field_elements(nonce);
    let aad_elems = bytes_to_field_elements(aad);
    let plaintext_elems = bytes_to_field_elements(plaintext);

    for e in &key_elems {
        sponge.absorb(e);
    }
    for e in &nonce_elems {
        sponge.absorb(e);
    }
    for e in &aad_elems {
        sponge.absorb(e);
    }

    let mut ciphertext_elems = Vec::with_capacity(plaintext_elems.len());
    for chunk in plaintext_elems.chunks(RATE) {
        let keystream: Vec<Fr> = sponge.squeeze_field_elements(chunk.len());
        for (pt, ks) in chunk.iter().zip(keystream.iter()) {
            ciphertext_elems.push(*pt + *ks);
        }
        for pt_elem in chunk {
            sponge.absorb(pt_elem);
        }
    }

    let tag_elems: Vec<Fr> = sponge.squeeze_field_elements(1);
    let tag = field_element_to_bytes_32(&tag_elems[0]);

    let ct_bytes = field_elements_to_bytes(&ciphertext_elems);
    let mut sealed = Vec::with_capacity(NONCE_LEN + ct_bytes.len() + TAG_LEN);
    sealed.extend_from_slice(nonce);
    sealed.extend_from_slice(&ct_bytes);
    sealed.extend_from_slice(&tag);
    Ok(sealed)
}

/// Poseidon sponge decryption.
///
/// Expects `sealed = nonce (32) || ciphertext_elements || tag (32)`.
pub fn poseidon_decrypt(
    sealed: &[u8],
    key: &SectorKey,
    nonce: &[u8; 32],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let min_len = NONCE_LEN + TAG_LEN;
    if sealed.len() < min_len {
        return Err(CryptoError::CiphertextTooShort {
            len: sealed.len(),
            min: min_len,
        });
    }

    let stored_nonce = &sealed[..NONCE_LEN];
    if stored_nonce != nonce {
        return Err(CryptoError::DecryptionFailed);
    }

    let ct_body = &sealed[NONCE_LEN..sealed.len() - TAG_LEN];
    let stored_tag = &sealed[sealed.len() - TAG_LEN..];

    let ciphertext_elems = bytes_to_field_elements_exact(ct_body)?;

    let mut sponge = PoseidonSponge::<Fr>::new(&POSEIDON_CONFIG);

    let key_elems = bytes_to_field_elements(key.as_bytes());
    let nonce_elems = bytes_to_field_elements(nonce);
    let aad_elems = bytes_to_field_elements(aad);

    for e in &key_elems {
        sponge.absorb(e);
    }
    for e in &nonce_elems {
        sponge.absorb(e);
    }
    for e in &aad_elems {
        sponge.absorb(e);
    }

    let mut plaintext_elems = Vec::with_capacity(ciphertext_elems.len());
    for chunk in ciphertext_elems.chunks(RATE) {
        let keystream: Vec<Fr> = sponge.squeeze_field_elements(chunk.len());
        let mut pt_chunk = Vec::with_capacity(chunk.len());
        for (ct, ks) in chunk.iter().zip(keystream.iter()) {
            pt_chunk.push(*ct - *ks);
        }
        for pt in &pt_chunk {
            sponge.absorb(pt);
        }
        plaintext_elems.extend(pt_chunk);
    }

    let tag_elems: Vec<Fr> = sponge.squeeze_field_elements(1);
    let expected_tag = field_element_to_bytes_32(&tag_elems[0]);

    if stored_tag != expected_tag.as_slice() {
        return Err(CryptoError::DecryptionFailed);
    }

    Ok(field_elements_to_original_bytes(&plaintext_elems))
}

/// Compute `Poseidon(data)` — a collision-resistant hash returning 32 bytes.
/// Used for ciphertext binding (the Zode computes this independently).
pub fn poseidon_hash(data: &[u8]) -> [u8; 32] {
    let mut sponge = PoseidonSponge::<Fr>::new(&POSEIDON_CONFIG);
    let elems = bytes_to_field_elements(data);
    for e in &elems {
        sponge.absorb(e);
    }
    let out: Vec<Fr> = sponge.squeeze_field_elements(1);
    field_element_to_bytes_32(&out[0])
}

/// Compute the Poseidon hash of the ciphertext field elements inside a
/// sealed blob (`nonce(32) || ct_elements || tag(32)`).
///
/// This matches the hash produced by the Groth16 prover/circuit, which
/// absorbs the raw Fr ciphertext elements — **not** the length-prefixed
/// byte packing used by [`poseidon_hash`].
pub fn poseidon_ciphertext_hash(sealed: &[u8]) -> Result<[u8; 32], CryptoError> {
    let min_len = NONCE_LEN + TAG_LEN;
    if sealed.len() < min_len {
        return Err(CryptoError::CiphertextTooShort {
            len: sealed.len(),
            min: min_len,
        });
    }
    let ct_body = &sealed[NONCE_LEN..sealed.len() - TAG_LEN];
    let ct_elems = bytes_to_field_elements_exact(ct_body)?;

    let mut sponge = PoseidonSponge::<Fr>::new(&POSEIDON_CONFIG);
    for e in &ct_elems {
        sponge.absorb(e);
    }
    let out: Vec<Fr> = sponge.squeeze_field_elements(1);
    Ok(field_element_to_bytes_32(&out[0]))
}

/// Encrypt plaintext for sector storage using Poseidon sponge.
/// Generates a random nonce internally.
/// AAD should be `program_id || sector_id`.
pub fn poseidon_encrypt_sector(
    plaintext: &[u8],
    key: &SectorKey,
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mut nonce = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut nonce);
    poseidon_encrypt(plaintext, key, &nonce, aad)
}

/// Decrypt sector ciphertext encrypted with [`poseidon_encrypt_sector`].
pub fn poseidon_decrypt_sector(
    sealed: &[u8],
    key: &SectorKey,
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::CiphertextTooShort {
            len: sealed.len(),
            min: NONCE_LEN + TAG_LEN,
        });
    }
    let nonce: [u8; 32] = sealed[..NONCE_LEN].try_into().unwrap();
    poseidon_decrypt(sealed, key, &nonce, aad)
}

// ---------------------------------------------------------------------------
// Field element ↔ byte conversion
// ---------------------------------------------------------------------------

/// Pack arbitrary bytes into BN254 field elements (31 usable bytes each).
/// Format per element: `[1-byte length][up to 31 bytes of data][zero-pad]`.
fn bytes_to_field_elements(data: &[u8]) -> Vec<Fr> {
    if data.is_empty() {
        return vec![Fr::from(0u64)];
    }
    let mut elems = Vec::with_capacity((data.len() + BYTES_PER_ELEMENT - 1) / BYTES_PER_ELEMENT);
    for chunk in data.chunks(BYTES_PER_ELEMENT) {
        let mut repr = [0u8; 32];
        repr[0] = chunk.len() as u8;
        repr[1..1 + chunk.len()].copy_from_slice(chunk);
        elems.push(Fr::from_le_bytes_mod_order(&repr));
    }
    elems
}

/// Reconstruct original bytes from field elements packed by `bytes_to_field_elements`.
fn field_elements_to_original_bytes(elems: &[Fr]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in elems {
        let repr = e.into_bigint().to_bytes_le();
        let len = repr[0] as usize;
        if len == 0 && out.is_empty() && elems.len() == 1 {
            return Vec::new();
        }
        if len > 0 && 1 + len <= repr.len() {
            out.extend_from_slice(&repr[1..1 + len]);
        }
    }
    out
}

/// Serialize field elements to bytes (32 bytes each, little-endian).
fn field_elements_to_bytes(elems: &[Fr]) -> Vec<u8> {
    let mut out = Vec::with_capacity(elems.len() * 32);
    for e in elems {
        let bytes = e.into_bigint().to_bytes_le();
        let mut buf = [0u8; 32];
        buf[..bytes.len()].copy_from_slice(&bytes);
        out.extend_from_slice(&buf);
    }
    out
}

/// Deserialize field elements from bytes (32 bytes each).
fn bytes_to_field_elements_exact(data: &[u8]) -> Result<Vec<Fr>, CryptoError> {
    if data.len() % 32 != 0 {
        return Err(CryptoError::PaddingError(format!(
            "ciphertext body length {} is not a multiple of 32",
            data.len()
        )));
    }
    let mut elems = Vec::with_capacity(data.len() / 32);
    for chunk in data.chunks_exact(32) {
        elems.push(Fr::from_le_bytes_mod_order(chunk));
    }
    Ok(elems)
}

fn field_element_to_bytes_32(e: &Fr) -> [u8; 32] {
    let bytes = e.into_bigint().to_bytes_le();
    let mut buf = [0u8; 32];
    buf[..bytes.len()].copy_from_slice(&bytes);
    buf
}

// ---------------------------------------------------------------------------
// Poseidon configuration (BN254, width=3, rate=2, capacity=1)
// ---------------------------------------------------------------------------

static POSEIDON_CONFIG: LazyLock<PoseidonConfig<Fr>> = LazyLock::new(|| {
    let full_rounds = 8;
    let partial_rounds = 57;
    let alpha = 5;
    let rate = RATE;
    let capacity = 1;
    let width = rate + capacity;

    let num_rounds = full_rounds + partial_rounds;
    let num_constants = num_rounds * width;
    let round_constants: Vec<Fr> = (0..num_constants)
        .map(|i| Fr::from((i + 1) as u64))
        .collect();

    let mds: Vec<Vec<Fr>> = (0..width)
        .map(|i| {
            (0..width)
                .map(|j| {
                    let val = if i == j { 2u64 } else { 1u64 };
                    Fr::from(val + (i * width + j) as u64)
                })
                .collect()
        })
        .collect();

    PoseidonConfig {
        full_rounds: full_rounds as usize,
        partial_rounds: partial_rounds as usize,
        alpha: alpha as u64,
        ark: round_constants.chunks(width).map(|c| c.to_vec()).collect(),
        mds,
        rate,
        capacity,
    }
});
