use crate::error::CryptoError;
use crate::signing::HybridSignature;

/// Multicodec prefix for Ed25519 public keys: 0xed01 (varint-encoded as two bytes).
const ED25519_MULTICODEC: [u8; 2] = [0xed, 0x01];

/// Encode an Ed25519 public key as a `did:key` string.
///
/// Format: `did:key:z` + base58btc(multicodec_prefix || public_key_bytes)
pub fn ed25519_to_did_key(pk: &[u8; 32]) -> String {
    let mut prefixed = Vec::with_capacity(2 + 32);
    prefixed.extend_from_slice(&ED25519_MULTICODEC);
    prefixed.extend_from_slice(pk);

    let encoded = bs58::encode(&prefixed).into_string();
    format!("did:key:z{encoded}")
}

/// Decode a `did:key` string to an Ed25519 public key (32 bytes).
///
/// Expects format: `did:key:z` + base58btc(0xed01 || 32-byte key)
pub fn did_key_to_ed25519(did: &str) -> Result<[u8; 32], CryptoError> {
    let multibase = did
        .strip_prefix("did:key:z")
        .ok_or_else(|| CryptoError::InvalidDid("must start with 'did:key:z'".into()))?;

    let decoded = bs58::decode(multibase)
        .into_vec()
        .map_err(|e| CryptoError::InvalidDid(format!("base58 decode failed: {e}")))?;

    if decoded.len() != 2 + 32 {
        return Err(CryptoError::InvalidDid(format!(
            "expected 34 bytes after base58, got {}",
            decoded.len()
        )));
    }

    if decoded[0] != ED25519_MULTICODEC[0] || decoded[1] != ED25519_MULTICODEC[1] {
        return Err(CryptoError::InvalidDid(
            "multicodec prefix is not Ed25519 (0xed01)".into(),
        ));
    }

    let mut pk = [0u8; 32];
    pk.copy_from_slice(&decoded[2..]);
    Ok(pk)
}

/// Verify the Ed25519 component of a hybrid signature using a `did:key` DID.
///
/// Extracts the Ed25519 public key from the DID and verifies only the
/// Ed25519 portion (first 64 bytes) of the serialised hybrid signature.
/// This is sufficient when the full ML-DSA verifying key is not available.
pub fn verify_did_ed25519(did: &str, msg: &[u8], sig_bytes: &[u8]) -> Result<(), CryptoError> {
    use ed25519_dalek::Verifier as _;

    if sig_bytes.len() < HybridSignature::ED25519_LEN {
        return Err(CryptoError::InvalidKeyLength {
            expected: HybridSignature::ED25519_LEN,
            got: sig_bytes.len(),
        });
    }

    let pk_bytes = did_key_to_ed25519(did)?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk_bytes)
        .map_err(|_| CryptoError::Ed25519VerifyFailed)?;

    let mut ed_sig_bytes = [0u8; 64];
    ed_sig_bytes.copy_from_slice(&sig_bytes[..64]);
    let ed_sig = ed25519_dalek::Signature::from_bytes(&ed_sig_bytes);

    vk.verify(msg, &ed_sig)
        .map_err(|_| CryptoError::Ed25519VerifyFailed)
}
