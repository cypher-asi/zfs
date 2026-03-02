use std::collections::HashMap;
use std::path::Path;

use ark_bn254::{Bn254, Fr};
use ark_crypto_primitives::sponge::{poseidon::PoseidonSponge, CryptographicSponge};
use ark_ff::{BigInteger, PrimeField};
use ark_groth16::Groth16;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_snark::SNARK;

use grid_core::{
    CiphertextHash, FieldSchema, ProgramId, ProofSystem, SchemaHash, SectorId, ShapeProof,
};
use grid_crypto::SectorKey;

use crate::circuit::{
    bytes_to_field_elements, default_poseidon_config, max_elements_for_bucket, ShapeEncryptCircuit,
};
use crate::error::Groth16Error;

/// Client-side Groth16 prover: encrypts with Poseidon sponge AND
/// generates the shape+encrypt proof in one operation.
pub struct Groth16ShapeProver {
    proving_keys: HashMap<u32, ark_groth16::ProvingKey<Bn254>>,
}

impl Groth16ShapeProver {
    /// Create a prover with an in-memory map of proving keys.
    pub fn from_keys(keys: HashMap<u32, ark_groth16::ProvingKey<Bn254>>) -> Self {
        Self { proving_keys: keys }
    }

    /// Load proving keys from a directory. Files named `shape_pk_{bucket}_{version}.bin`.
    pub fn load(pk_dir: &Path) -> Result<Self, Groth16Error> {
        let ver = crate::KEY_VERSION;
        let mut keys = HashMap::new();
        for bucket in &[1024u32, 4096] {
            let path = pk_dir.join(format!("shape_pk_{bucket}_{ver}.bin"));
            if path.exists() {
                let data = std::fs::read(&path)
                    .map_err(|e| Groth16Error::SerializationError(e.to_string()))?;
                let pk = ark_groth16::ProvingKey::<Bn254>::deserialize_compressed(&data[..])
                    .map_err(|e| Groth16Error::SerializationError(e.to_string()))?;
                keys.insert(*bucket, pk);
            }
        }
        Ok(Self { proving_keys: keys })
    }

    /// Select the smallest bucket that fits the plaintext.
    fn select_bucket(&self, plaintext_len: usize) -> Result<u32, Groth16Error> {
        let mut buckets: Vec<u32> = self.proving_keys.keys().copied().collect();
        buckets.sort();
        for b in buckets {
            if plaintext_len <= b as usize {
                return Ok(b);
            }
        }
        Err(Groth16Error::PlaintextTooLarge {
            len: plaintext_len,
            max: self.proving_keys.keys().max().copied().unwrap_or(0) as usize,
        })
    }

    /// Encrypt plaintext with Poseidon sponge AND generate the Groth16 proof.
    ///
    /// Returns `(sealed_ciphertext, ShapeProof)`.
    pub fn encrypt_and_prove(
        &self,
        plaintext_padded: &[u8],
        key: &SectorKey,
        program_id: &ProgramId,
        sector_id: &SectorId,
        schema: &FieldSchema,
    ) -> Result<(Vec<u8>, ShapeProof), Groth16Error> {
        let bucket = self.select_bucket(plaintext_padded.len())?;
        let pk = self
            .proving_keys
            .get(&bucket)
            .ok_or(Groth16Error::InvalidBucketSize { size: bucket })?;

        let config = default_poseidon_config();

        let mut nonce = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);

        let mut aad = Vec::with_capacity(program_id.as_bytes().len() + sector_id.as_bytes().len());
        aad.extend_from_slice(program_id.as_bytes());
        aad.extend_from_slice(sector_id.as_bytes());

        let key_elems = bytes_to_field_elements(key.as_bytes());
        let nonce_elems = bytes_to_field_elements(&nonce);
        let aad_elems = bytes_to_field_elements(&aad);

        let max_elems = max_elements_for_bucket(bucket);
        let mut plaintext_elems = bytes_to_field_elements(plaintext_padded);
        plaintext_elems.resize(max_elems, Fr::from(0u64));

        let mut sponge = PoseidonSponge::<Fr>::new(&config);
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
        for chunk in plaintext_elems.chunks(2) {
            let keystream: Vec<Fr> = sponge.squeeze_field_elements(chunk.len());
            for (pt, ks) in chunk.iter().zip(keystream.iter()) {
                ciphertext_elems.push(*pt + *ks);
            }
            for pt in chunk {
                sponge.absorb(pt);
            }
        }

        let tag_elems: Vec<Fr> = sponge.squeeze_field_elements(1);
        let tag = field_element_to_bytes_32(&tag_elems[0]);

        let ct_bytes = field_elements_to_bytes(&ciphertext_elems);
        let mut sealed = Vec::with_capacity(32 + ct_bytes.len() + 32);
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ct_bytes);
        sealed.extend_from_slice(&tag);

        let mut ct_sponge = PoseidonSponge::<Fr>::new(&config);
        for ct in &ciphertext_elems {
            ct_sponge.absorb(ct);
        }
        let ct_hash_elems: Vec<Fr> = ct_sponge.squeeze_field_elements(1);
        let ciphertext_hash_fr = ct_hash_elems[0];
        let ciphertext_hash = field_element_to_bytes_32(&ciphertext_hash_fr);

        let schema_hash = schema.schema_hash();
        let schema_hash_fr = Fr::from_le_bytes_mod_order(&schema_hash);

        let circuit = ShapeEncryptCircuit {
            plaintext_elems,
            key_elems,
            nonce_elems,
            aad_elems,
            ciphertext_hash: ciphertext_hash_fr,
            schema_hash: schema_hash_fr,
            poseidon_config: config,
        };

        let mut rng = rand::thread_rng();
        let proof = Groth16::<Bn254>::prove(pk, circuit, &mut rng)
            .map_err(|e| Groth16Error::ProvingFailed(e.to_string()))?;

        let mut proof_bytes = Vec::new();
        proof
            .serialize_compressed(&mut proof_bytes)
            .map_err(|e| Groth16Error::SerializationError(e.to_string()))?;

        let shape_proof = ShapeProof {
            proof_system: ProofSystem::Groth16,
            ciphertext_hash: CiphertextHash(ciphertext_hash.to_vec()),
            proof_bytes,
            schema_hash: SchemaHash(schema_hash.to_vec()),
            size_bucket: bucket,
        };

        Ok((sealed, shape_proof))
    }
}

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

fn field_element_to_bytes_32(e: &Fr) -> [u8; 32] {
    let bytes = e.into_bigint().to_bytes_le();
    let mut buf = [0u8; 32];
    buf[..bytes.len()].copy_from_slice(&bytes);
    buf
}
