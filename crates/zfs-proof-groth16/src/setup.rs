use ark_bn254::{Bn254, Fr};
use ark_groth16::Groth16;
use ark_snark::SNARK;
use rand::SeedableRng;

use crate::circuit::{
    bytes_to_field_elements, default_poseidon_config, max_elements_for_bucket, ShapeEncryptCircuit,
};
use crate::error::Groth16Error;

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
