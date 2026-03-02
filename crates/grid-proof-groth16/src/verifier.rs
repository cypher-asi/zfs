use std::collections::HashMap;
use std::path::Path;

use ark_bn254::{Bn254, Fr};
use ark_ff::PrimeField;
use ark_groth16::{Groth16, PreparedVerifyingKey};
use ark_serialize::CanonicalDeserialize;
use ark_snark::SNARK;

use grid_core::{Cid, ProgramId};
use grid_proof::{ProofError, ProofVerifier, VerifiedSector};

use crate::error::Groth16Error;

/// Zode-side Groth16 shape verifier.
///
/// Holds one prepared verifying key per message-size bucket.
pub struct Groth16ShapeVerifier {
    verifying_keys: HashMap<u32, PreparedVerifyingKey<Bn254>>,
}

impl Groth16ShapeVerifier {
    /// Build from pre-loaded verifying keys.
    pub fn from_keys(keys: HashMap<u32, ark_groth16::VerifyingKey<Bn254>>) -> Self {
        let prepared = keys
            .into_iter()
            .map(|(bucket, vk)| (bucket, ark_groth16::prepare_verifying_key(&vk)))
            .collect();
        Self {
            verifying_keys: prepared,
        }
    }

    /// Load verifying keys from a directory. Files named `shape_vk_{bucket}_{version}.bin`.
    pub fn load(vk_dir: &Path) -> Result<Self, Groth16Error> {
        let ver = crate::KEY_VERSION;
        let mut keys = HashMap::new();
        for bucket in &[1024u32, 4096] {
            let path = vk_dir.join(format!("shape_vk_{bucket}_{ver}.bin"));
            if path.exists() {
                let data = std::fs::read(&path)
                    .map_err(|e| Groth16Error::SerializationError(e.to_string()))?;
                let vk = ark_groth16::VerifyingKey::<Bn254>::deserialize_compressed(&data[..])
                    .map_err(|e| Groth16Error::SerializationError(e.to_string()))?;
                keys.insert(*bucket, vk);
            }
        }
        Ok(Self::from_keys(keys))
    }
}

impl ProofVerifier for Groth16ShapeVerifier {
    /// Verify a Groth16 shape+encrypt proof.
    ///
    /// `payload_context` must contain:
    /// `ciphertext_hash (32) || schema_hash (32) || size_bucket (4, LE)`.
    fn verify(
        &self,
        cid: &Cid,
        program_id: &ProgramId,
        version: u64,
        proof: &[u8],
        payload_context: Option<&[u8]>,
    ) -> Result<VerifiedSector, ProofError> {
        let ctx = payload_context.ok_or_else(|| ProofError::InvalidProofFormat {
            reason: "missing payload_context for Groth16 verification".into(),
        })?;

        if ctx.len() < 68 {
            return Err(ProofError::InvalidProofFormat {
                reason: format!(
                    "payload_context too short: {} bytes, need at least 68",
                    ctx.len()
                ),
            });
        }

        let ct_hash_bytes = &ctx[..32];
        let schema_hash_bytes = &ctx[32..64];
        let bucket_bytes: [u8; 4] =
            ctx[64..68]
                .try_into()
                .map_err(|_| ProofError::InvalidProofFormat {
                    reason: "invalid bucket bytes".into(),
                })?;
        let size_bucket = u32::from_le_bytes(bucket_bytes);

        let pvk = self.verifying_keys.get(&size_bucket).ok_or_else(|| {
            ProofError::VerifierNotFound {
                proof_system: format!("groth16_bucket_{size_bucket}"),
            }
        })?;

        let groth16_proof =
            ark_groth16::Proof::<Bn254>::deserialize_compressed(proof).map_err(|e| {
                ProofError::InvalidProofFormat {
                    reason: format!("failed to deserialize Groth16 proof: {e}"),
                }
            })?;

        let ct_hash_fr = Fr::from_le_bytes_mod_order(ct_hash_bytes);
        let schema_hash_fr = Fr::from_le_bytes_mod_order(schema_hash_bytes);
        let public_inputs = vec![ct_hash_fr, schema_hash_fr];

        let valid = Groth16::<Bn254>::verify_with_processed_vk(pvk, &public_inputs, &groth16_proof)
            .map_err(|e| ProofError::VerificationFailed {
                reason: format!("Groth16 verify error: {e}"),
            })?;

        if !valid {
            return Err(ProofError::VerificationFailed {
                reason: "Groth16 proof did not verify".into(),
            });
        }

        Ok(VerifiedSector {
            cid: *cid,
            program_id: *program_id,
            version,
        })
    }
}
