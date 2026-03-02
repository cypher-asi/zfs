use std::path::Path;

use ark_bn254::{Bn254, Fr};
use ark_groth16::Groth16;
use ark_serialize::CanonicalSerialize;
use ark_snark::SNARK;
use rand::SeedableRng;

use crate::circuit::{
    bytes_to_field_elements, default_poseidon_config, max_elements_for_bucket, ShapeEncryptCircuit,
    BUCKET_1K, BUCKET_4K,
};
use crate::error::Groth16Error;

const PROOF_BUCKETS: &[u32] = &[1024, 4096];

/// Key-file version suffix. Bump whenever the circuit structure changes
/// (e.g. BYTES_PER_ELEMENT, sponge order) so stale cached keys are ignored.
pub const KEY_VERSION: &str = "v2";

/// Deterministic seed for the Groth16 trusted setup.
///
/// All zodes must derive the same PK/VK pair so that proofs generated on
/// one node can be verified by any other. A fixed seed makes this
/// reproducible. In production this would be replaced by a proper
/// multi-party trusted setup ceremony.
const SETUP_SEED: [u8; 32] = *b"zfs-groth16-trusted-setup-v0002\0";

/// Generate proving and verifying keys for a given message-size bucket.
///
/// Uses a dummy witness of the correct size to determine constraint count.
/// The RNG is seeded deterministically (per-bucket) so every node produces
/// identical keys.
pub fn generate_keys_for_bucket(
    bucket_size: u32,
) -> Result<
    (
        ark_groth16::ProvingKey<Bn254>,
        ark_groth16::VerifyingKey<Bn254>,
    ),
    Groth16Error,
> {
    if bucket_size != BUCKET_1K && bucket_size != BUCKET_4K {
        return Err(Groth16Error::InvalidBucketSize { size: bucket_size });
    }
    let max_elems = max_elements_for_bucket(bucket_size);
    let dummy_plaintext = vec![Fr::from(0u64); max_elems];
    let dummy_key = bytes_to_field_elements(&[0u8; 32]);
    let dummy_nonce = bytes_to_field_elements(&[0u8; 32]);
    let dummy_aad = bytes_to_field_elements(&[0u8; 64]);
    let config = default_poseidon_config();

    let circuit = ShapeEncryptCircuit {
        plaintext_elems: dummy_plaintext,
        key_elems: dummy_key,
        nonce_elems: dummy_nonce,
        aad_elems: dummy_aad,
        ciphertext_hash: Fr::from(0u64),
        schema_hash: Fr::from(0u64),
        poseidon_config: config,
    };

    let mut bucket_seed = SETUP_SEED;
    let bucket_bytes = bucket_size.to_le_bytes();
    for (i, b) in bucket_bytes.iter().enumerate() {
        bucket_seed[28 + i] ^= b;
    }
    let mut rng = rand::rngs::StdRng::from_seed(bucket_seed);
    let (pk, vk) = Groth16::<Bn254>::circuit_specific_setup(circuit, &mut rng)
        .map_err(|e| Groth16Error::SetupFailed(e.to_string()))?;

    Ok((pk, vk))
}

/// Ensure Groth16 proving and verifying key files exist in `key_dir`.
///
/// Generates missing keys on a thread with an 8 MB stack (required by the
/// arkworks constraint system). Returns immediately when all files are
/// already present.
pub fn ensure_keys(key_dir: &Path) -> Result<(), Groth16Error> {
    let ver = KEY_VERSION;
    std::fs::create_dir_all(key_dir)
        .map_err(|e| Groth16Error::SetupFailed(format!("create key dir: {e}")))?;

    let all_exist = PROOF_BUCKETS.iter().all(|b| {
        key_dir.join(format!("shape_pk_{b}_{ver}.bin")).exists()
            && key_dir.join(format!("shape_vk_{b}_{ver}.bin")).exists()
    });
    if all_exist {
        return Ok(());
    }

    let dir = key_dir.to_path_buf();
    let handle = std::thread::Builder::new()
        .name("groth16-keygen".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || -> Result<(), Groth16Error> {
            for &bucket in PROOF_BUCKETS {
                let pk_path = dir.join(format!("shape_pk_{bucket}_{ver}.bin"));
                let vk_path = dir.join(format!("shape_vk_{bucket}_{ver}.bin"));
                if pk_path.exists() && vk_path.exists() {
                    continue;
                }
                let (pk, vk) = generate_keys_for_bucket(bucket)?;

                let mut pk_bytes = Vec::new();
                pk.serialize_compressed(&mut pk_bytes)
                    .map_err(|e| Groth16Error::SerializationError(format!("PK: {e}")))?;
                std::fs::write(&pk_path, &pk_bytes)
                    .map_err(|e| Groth16Error::SetupFailed(format!("write PK: {e}")))?;

                let mut vk_bytes = Vec::new();
                vk.serialize_compressed(&mut vk_bytes)
                    .map_err(|e| Groth16Error::SerializationError(format!("VK: {e}")))?;
                std::fs::write(&vk_path, &vk_bytes)
                    .map_err(|e| Groth16Error::SetupFailed(format!("write VK: {e}")))?;
            }
            Ok(())
        })
        .map_err(|e| Groth16Error::SetupFailed(format!("spawn keygen thread: {e}")))?;

    handle
        .join()
        .map_err(|_| Groth16Error::SetupFailed("keygen thread panicked".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_keys_invalid_bucket_size() {
        let result = generate_keys_for_bucket(2048);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, Groth16Error::InvalidBucketSize { size: 2048 }),
            "expected InvalidBucketSize(2048), got: {err}"
        );
    }
}
