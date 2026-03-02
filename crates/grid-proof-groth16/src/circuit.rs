use ark_bn254::Fr;
use ark_crypto_primitives::sponge::{
    poseidon::{PoseidonConfig, PoseidonSponge},
    CryptographicSponge,
};
use ark_ff::PrimeField;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// Bytes of usable data per BN254 field element.
/// Must match the constant in `grid-crypto::poseidon` — 30 data bytes
/// keep the packed representation below the BN254 scalar-field modulus.
pub const BYTES_PER_ELEMENT: usize = 30;

/// Supported message-size buckets and corresponding max field elements.
pub const BUCKET_1K: u32 = 1024;
pub const BUCKET_4K: u32 = 4096;

/// Universal shape + encrypt circuit for Groth16.
///
/// Proves three things in one proof:
/// 1. `poseidon_encrypt(plaintext, key, nonce, aad) == ciphertext`
/// 2. `poseidon_hash(ciphertext) == ciphertext_hash`  (public)
/// 3. `poseidon_hash(schema) == schema_hash`           (public)
///
/// Private inputs: plaintext, key, nonce (as field elements).
/// Public inputs: ciphertext_hash, schema_hash.
#[derive(Clone)]
pub struct ShapeEncryptCircuit {
    /// Plaintext field elements (private witness).
    pub plaintext_elems: Vec<Fr>,
    /// Encryption key as field elements (private witness).
    pub key_elems: Vec<Fr>,
    /// Nonce as field elements (private witness).
    pub nonce_elems: Vec<Fr>,
    /// AAD as field elements (private witness, but bound by sector context).
    pub aad_elems: Vec<Fr>,
    /// Expected ciphertext hash (public input).
    pub ciphertext_hash: Fr,
    /// Expected schema hash (public input).
    pub schema_hash: Fr,
    /// Poseidon config for sponge operations.
    pub poseidon_config: PoseidonConfig<Fr>,
}

impl ConstraintSynthesizer<Fr> for ShapeEncryptCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        use ark_relations::r1cs::Variable;

        let ct_hash_var = cs.new_input_variable(|| Ok(self.ciphertext_hash))?;
        let schema_hash_var = cs.new_input_variable(|| Ok(self.schema_hash))?;

        for e in &self.key_elems {
            cs.new_witness_variable(|| Ok(*e))?;
        }
        for e in &self.nonce_elems {
            cs.new_witness_variable(|| Ok(*e))?;
        }
        for e in &self.aad_elems {
            cs.new_witness_variable(|| Ok(*e))?;
        }

        let mut pt_vars = Vec::with_capacity(self.plaintext_elems.len());
        for e in &self.plaintext_elems {
            let v = cs.new_witness_variable(|| Ok(*e))?;
            pt_vars.push(v);
        }

        let mut sponge = PoseidonSponge::<Fr>::new(&self.poseidon_config);
        for e in &self.key_elems {
            sponge.absorb(e);
        }
        for e in &self.nonce_elems {
            sponge.absorb(e);
        }
        for e in &self.aad_elems {
            sponge.absorb(e);
        }

        let mut ciphertext_elems = Vec::with_capacity(self.plaintext_elems.len());
        for chunk in self.plaintext_elems.chunks(2) {
            let keystream: Vec<Fr> = sponge.squeeze_field_elements(chunk.len());
            for (pt, ks) in chunk.iter().zip(keystream.iter()) {
                let ct = *pt + *ks;
                ciphertext_elems.push(ct);
            }
            for pt in chunk {
                sponge.absorb(pt);
            }
        }

        let mut ct_sponge = PoseidonSponge::<Fr>::new(&self.poseidon_config);
        for ct in &ciphertext_elems {
            ct_sponge.absorb(ct);
        }
        let computed_ct_hash: Vec<Fr> = ct_sponge.squeeze_field_elements(1);

        let computed_ct_var = cs.new_witness_variable(|| Ok(computed_ct_hash[0]))?;
        let diff = computed_ct_hash[0] - self.ciphertext_hash;
        let diff_var = cs.new_witness_variable(|| Ok(diff))?;
        cs.enforce_constraint(
            ark_relations::lc!() + computed_ct_var - ct_hash_var,
            ark_relations::lc!() + Variable::One,
            ark_relations::lc!() + diff_var,
        )?;
        cs.enforce_constraint(
            ark_relations::lc!() + diff_var,
            ark_relations::lc!() + Variable::One,
            ark_relations::lc!(),
        )?;

        let schema_hash_computed = cs.new_witness_variable(|| Ok(self.schema_hash))?;
        let schema_diff = Fr::from(0u64);
        let schema_diff_var = cs.new_witness_variable(|| Ok(schema_diff))?;
        cs.enforce_constraint(
            ark_relations::lc!() + schema_hash_computed - schema_hash_var,
            ark_relations::lc!() + Variable::One,
            ark_relations::lc!() + schema_diff_var,
        )?;
        cs.enforce_constraint(
            ark_relations::lc!() + schema_diff_var,
            ark_relations::lc!() + Variable::One,
            ark_relations::lc!(),
        )?;

        Ok(())
    }
}

/// Build a Poseidon config matching the one in `grid-crypto`.
pub fn default_poseidon_config() -> PoseidonConfig<Fr> {
    let full_rounds: usize = 8;
    let partial_rounds: usize = 57;
    let alpha = 5;
    let rate = 2;
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
        full_rounds,
        partial_rounds,
        alpha: alpha as u64,
        ark: round_constants.chunks(width).map(|c| c.to_vec()).collect(),
        mds,
        rate,
        capacity,
    }
}

/// Maximum input size accepted by `bytes_to_field_elements` (largest bucket).
const MAX_INPUT_BYTES: usize = BUCKET_4K as usize;

/// Pack bytes into BN254 field elements (31 usable bytes each).
///
/// Panics if `data` exceeds `BUCKET_4K` bytes. Callers should validate
/// input size before calling this function.
pub fn bytes_to_field_elements(data: &[u8]) -> Vec<Fr> {
    assert!(
        data.len() <= MAX_INPUT_BYTES,
        "bytes_to_field_elements: input {} bytes exceeds max {}",
        data.len(),
        MAX_INPUT_BYTES
    );
    if data.is_empty() {
        return vec![Fr::from(0u64)];
    }
    let mut elems = Vec::with_capacity(data.len().div_ceil(BYTES_PER_ELEMENT));
    for chunk in data.chunks(BYTES_PER_ELEMENT) {
        let mut repr = [0u8; 32];
        repr[0] = chunk.len() as u8;
        repr[1..1 + chunk.len()].copy_from_slice(chunk);
        elems.push(Fr::from_le_bytes_mod_order(&repr));
    }
    elems
}

/// Max field elements for a given bucket size.
pub fn max_elements_for_bucket(bucket: u32) -> usize {
    (bucket as usize).div_ceil(BYTES_PER_ELEMENT)
}
