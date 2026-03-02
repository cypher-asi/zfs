use ark_bn254::Fr;
use ark_r1cs_std::{alloc::AllocVar, eq::EqGadget, fields::fp::FpVar};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// All witness values needed to prove a spend transaction.
pub struct SpendWitness {
    pub owner_secret: Fr,
    pub input_value: Fr,
    pub input_randomness: Fr,
    pub output_values: Vec<Fr>,
    pub output_randomnesses: Vec<Fr>,
    pub output_owner_pubkeys: Vec<Fr>,
    pub input_commitment: Fr,
    pub nullifier: Fr,
    pub output_commitments: Vec<Fr>,
}

/// R1CS constraints for a Zephyr spend transaction.
///
/// Constraints enforced:
/// 1. `input_commitment == hash(input_value, owner_pubkey, input_randomness)`
/// 2. `nullifier == hash(owner_secret, input_commitment)`
/// 3. `input_value == sum(output_values)` (value conservation)
/// 4. For each output: `output_commitment_i == hash(output_value_i, output_owner_pubkey_i, output_randomness_i)`
///
/// Uses a simplified hash (multiplication-based) for the MVP circuit.
/// In production this would be replaced with Poseidon sponge constraints
/// from `ark-crypto-primitives`.
#[derive(Clone)]
pub struct SpendCircuit {
    pub owner_secret: Option<Fr>,
    pub input_value: Option<Fr>,
    pub input_randomness: Option<Fr>,
    pub output_values: Vec<Option<Fr>>,
    pub output_randomnesses: Vec<Option<Fr>>,
    pub output_owner_pubkeys: Vec<Option<Fr>>,

    pub input_commitment: Option<Fr>,
    pub nullifier: Option<Fr>,
    pub output_commitments: Vec<Option<Fr>>,
}

impl SpendCircuit {
    /// Create a circuit with concrete witness values for proving.
    pub fn new(witness: SpendWitness) -> Self {
        Self {
            owner_secret: Some(witness.owner_secret),
            input_value: Some(witness.input_value),
            input_randomness: Some(witness.input_randomness),
            output_values: witness.output_values.into_iter().map(Some).collect(),
            output_randomnesses: witness.output_randomnesses.into_iter().map(Some).collect(),
            output_owner_pubkeys: witness.output_owner_pubkeys.into_iter().map(Some).collect(),
            input_commitment: Some(witness.input_commitment),
            nullifier: Some(witness.nullifier),
            output_commitments: witness.output_commitments.into_iter().map(Some).collect(),
        }
    }

    /// Create a blank circuit for trusted setup (no witness).
    pub fn blank(num_outputs: usize) -> Self {
        Self {
            owner_secret: None,
            input_value: None,
            input_randomness: None,
            output_values: vec![None; num_outputs],
            output_randomnesses: vec![None; num_outputs],
            output_owner_pubkeys: vec![None; num_outputs],
            input_commitment: None,
            nullifier: None,
            output_commitments: vec![None; num_outputs],
        }
    }
}

/// Simplified hash: `hash(a, b, c) = a * b + c + 1`
///
/// This is a placeholder for Poseidon sponge constraints. The structure
/// is correct (R1CS with multiplication gates); only the hash function
/// differs from production.
fn simplified_hash(
    cs: ConstraintSystemRef<Fr>,
    a: &FpVar<Fr>,
    b: &FpVar<Fr>,
    c: &FpVar<Fr>,
) -> Result<FpVar<Fr>, SynthesisError> {
    let one = FpVar::new_constant(cs, Fr::from(1u64))?;
    let ab = a * b;
    Ok(&ab + c + &one)
}

/// Simplified 2-input hash: `hash2(a, b) = a * b + 1`
fn simplified_hash2(
    cs: ConstraintSystemRef<Fr>,
    a: &FpVar<Fr>,
    b: &FpVar<Fr>,
) -> Result<FpVar<Fr>, SynthesisError> {
    let one = FpVar::new_constant(cs, Fr::from(1u64))?;
    let ab = a * b;
    Ok(&ab + &one)
}

impl ConstraintSynthesizer<Fr> for SpendCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        let num_outputs = self.output_values.len();

        let owner_secret_var = FpVar::new_witness(cs.clone(), || {
            self.owner_secret.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let input_value_var = FpVar::new_witness(cs.clone(), || {
            self.input_value.ok_or(SynthesisError::AssignmentMissing)
        })?;
        let input_randomness_var = FpVar::new_witness(cs.clone(), || {
            self.input_randomness
                .ok_or(SynthesisError::AssignmentMissing)
        })?;

        let input_commitment_var = FpVar::new_input(cs.clone(), || {
            self.input_commitment
                .ok_or(SynthesisError::AssignmentMissing)
        })?;
        let nullifier_var = FpVar::new_input(cs.clone(), || {
            self.nullifier.ok_or(SynthesisError::AssignmentMissing)
        })?;

        // owner_pubkey = owner_secret (simplified; in production: Poseidon(owner_secret))
        let owner_pubkey_var = &owner_secret_var;

        // Constraint 1: input_commitment == hash(input_value, owner_pubkey, input_randomness)
        let computed_commitment = simplified_hash(
            cs.clone(),
            &input_value_var,
            owner_pubkey_var,
            &input_randomness_var,
        )?;
        computed_commitment.enforce_equal(&input_commitment_var)?;

        // Constraint 2: nullifier == hash2(owner_secret, input_commitment)
        let computed_nullifier =
            simplified_hash2(cs.clone(), &owner_secret_var, &input_commitment_var)?;
        computed_nullifier.enforce_equal(&nullifier_var)?;

        // Constraint 3: value conservation
        let mut output_sum = FpVar::new_constant(cs.clone(), Fr::from(0u64))?;

        for i in 0..num_outputs {
            let out_val = FpVar::new_witness(cs.clone(), || {
                self.output_values[i].ok_or(SynthesisError::AssignmentMissing)
            })?;
            let out_rand = FpVar::new_witness(cs.clone(), || {
                self.output_randomnesses[i].ok_or(SynthesisError::AssignmentMissing)
            })?;
            let out_pubkey = FpVar::new_witness(cs.clone(), || {
                self.output_owner_pubkeys[i].ok_or(SynthesisError::AssignmentMissing)
            })?;

            let out_commitment_var = FpVar::new_input(cs.clone(), || {
                self.output_commitments[i].ok_or(SynthesisError::AssignmentMissing)
            })?;

            // Constraint 4: output_commitment_i == hash(output_value_i, output_owner_pubkey_i, output_randomness_i)
            let computed_out = simplified_hash(cs.clone(), &out_val, &out_pubkey, &out_rand)?;
            computed_out.enforce_equal(&out_commitment_var)?;

            output_sum = &output_sum + &out_val;
        }

        input_value_var.enforce_equal(&output_sum)?;

        Ok(())
    }
}

/// Compute the simplified hash for witness generation.
pub fn compute_commitment(value: Fr, owner_pubkey: Fr, randomness: Fr) -> Fr {
    value * owner_pubkey + randomness + Fr::from(1u64)
}

/// Compute the simplified nullifier for witness generation.
pub fn compute_nullifier(owner_secret: Fr, commitment: Fr) -> Fr {
    owner_secret * commitment + Fr::from(1u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;

    fn make_witness_1output(
        owner_secret: Fr,
        input_value: Fr,
        input_randomness: Fr,
        out_value: Fr,
        out_rand: Fr,
        out_pubkey: Fr,
    ) -> SpendWitness {
        let commitment = compute_commitment(input_value, owner_secret, input_randomness);
        let nullifier = compute_nullifier(owner_secret, commitment);
        let out_commitment = compute_commitment(out_value, out_pubkey, out_rand);
        SpendWitness {
            owner_secret,
            input_value,
            input_randomness,
            output_values: vec![out_value],
            output_randomnesses: vec![out_rand],
            output_owner_pubkeys: vec![out_pubkey],
            input_commitment: commitment,
            nullifier,
            output_commitments: vec![out_commitment],
        }
    }

    #[test]
    fn valid_spend_satisfies_constraints() {
        let w = make_witness_1output(
            Fr::from(42u64),
            Fr::from(100u64),
            Fr::from(7u64),
            Fr::from(100u64),
            Fr::from(13u64),
            Fr::from(99u64),
        );
        let circuit = SpendCircuit::new(w);
        let cs = ConstraintSystem::<Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(cs.is_satisfied().unwrap());
    }

    #[test]
    fn wrong_nullifier_fails() {
        let mut w = make_witness_1output(
            Fr::from(42u64),
            Fr::from(100u64),
            Fr::from(7u64),
            Fr::from(100u64),
            Fr::from(1u64),
            Fr::from(2u64),
        );
        w.nullifier = Fr::from(999u64);
        let circuit = SpendCircuit::new(w);
        let cs = ConstraintSystem::<Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(!cs.is_satisfied().unwrap());
    }

    #[test]
    fn value_mismatch_fails() {
        let owner_secret = Fr::from(42u64);
        let input_value = Fr::from(100u64);
        let input_randomness = Fr::from(7u64);
        let commitment = compute_commitment(input_value, owner_secret, input_randomness);
        let nullifier = compute_nullifier(owner_secret, commitment);
        let out_value = Fr::from(50u64);
        let out_rand = Fr::from(1u64);
        let out_pubkey = Fr::from(2u64);

        let w = SpendWitness {
            owner_secret,
            input_value,
            input_randomness,
            output_values: vec![out_value],
            output_randomnesses: vec![out_rand],
            output_owner_pubkeys: vec![out_pubkey],
            input_commitment: commitment,
            nullifier,
            output_commitments: vec![compute_commitment(out_value, out_pubkey, out_rand)],
        };
        let circuit = SpendCircuit::new(w);
        let cs = ConstraintSystem::<Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(!cs.is_satisfied().unwrap());
    }

    #[test]
    fn multi_output_value_conservation() {
        let owner_secret = Fr::from(10u64);
        let input_value = Fr::from(100u64);
        let input_randomness = Fr::from(5u64);
        let commitment = compute_commitment(input_value, owner_secret, input_randomness);
        let nullifier = compute_nullifier(owner_secret, commitment);

        let (v1, r1, p1) = (Fr::from(60u64), Fr::from(1u64), Fr::from(20u64));
        let (v2, r2, p2) = (Fr::from(40u64), Fr::from(2u64), Fr::from(30u64));

        let w = SpendWitness {
            owner_secret,
            input_value,
            input_randomness,
            output_values: vec![v1, v2],
            output_randomnesses: vec![r1, r2],
            output_owner_pubkeys: vec![p1, p2],
            input_commitment: commitment,
            nullifier,
            output_commitments: vec![
                compute_commitment(v1, p1, r1),
                compute_commitment(v2, p2, r2),
            ],
        };
        let circuit = SpendCircuit::new(w);
        let cs = ConstraintSystem::<Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).unwrap();
        assert!(cs.is_satisfied().unwrap());
    }
}
