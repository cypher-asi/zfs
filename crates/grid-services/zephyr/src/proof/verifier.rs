use ark_bn254::{Bn254, Fr};
use ark_groth16::{Groth16, PreparedVerifyingKey, VerifyingKey};
use ark_serialize::CanonicalDeserialize;
use ark_snark::SNARK;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerifierError {
    #[error("invalid proof format: {0}")]
    InvalidFormat(String),
    #[error("proof verification failed")]
    VerificationFailed,
    #[error("insufficient public signals: expected at least {expected}, got {got}")]
    InsufficientSignals { expected: usize, got: usize },
}

/// Verifies Groth16 spend proofs against a fixed verifying key.
pub struct SpendProofVerifier {
    pvk: PreparedVerifyingKey<Bn254>,
}

impl SpendProofVerifier {
    pub fn new(verifying_key: VerifyingKey<Bn254>) -> Self {
        let pvk = Groth16::<Bn254>::process_vk(&verifying_key)
            .expect("verifying key processing should not fail");
        Self { pvk }
    }

    /// Verify a spend proof against public signals.
    ///
    /// `public_signals` must contain at least 2 elements:
    /// `[input_commitment, nullifier, output_commitment_0, ...]`
    pub fn verify(&self, proof_bytes: &[u8], public_signals: &[Fr]) -> Result<(), VerifierError> {
        if public_signals.len() < 2 {
            return Err(VerifierError::InsufficientSignals {
                expected: 2,
                got: public_signals.len(),
            });
        }

        let proof = ark_groth16::Proof::<Bn254>::deserialize_compressed(proof_bytes)
            .map_err(|e| VerifierError::InvalidFormat(e.to_string()))?;

        let valid = Groth16::<Bn254>::verify_with_processed_vk(&self.pvk, public_signals, &proof)
            .map_err(|_| VerifierError::VerificationFailed)?;

        if valid {
            Ok(())
        } else {
            Err(VerifierError::VerificationFailed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof::circuit::{compute_commitment, compute_nullifier, SpendCircuit};
    use crate::proof::prover::SpendProver;
    use ark_groth16::Groth16;
    use ark_snark::SNARK;

    fn setup_keys() -> (ark_groth16::ProvingKey<Bn254>, VerifyingKey<Bn254>) {
        let blank = SpendCircuit::blank(1);
        let mut rng = rand::rngs::OsRng;
        Groth16::<Bn254>::circuit_specific_setup(blank, &mut rng).unwrap()
    }

    #[test]
    fn valid_proof_verifies() {
        let (pk, vk) = setup_keys();
        let prover = SpendProver::new(pk);
        let verifier = SpendProofVerifier::new(vk);

        let owner_secret = Fr::from(42u64);
        let input_value = Fr::from(100u64);
        let input_randomness = Fr::from(7u64);
        let out_value = Fr::from(100u64);
        let out_rand = Fr::from(13u64);
        let out_pubkey = Fr::from(99u64);

        let (proof_bytes, signals) = prover
            .prove(
                owner_secret,
                input_value,
                input_randomness,
                vec![out_value],
                vec![out_rand],
                vec![out_pubkey],
            )
            .unwrap();

        assert!(verifier.verify(&proof_bytes, &signals).is_ok());
    }

    #[test]
    fn tampered_signal_fails() {
        let (pk, vk) = setup_keys();
        let prover = SpendProver::new(pk);
        let verifier = SpendProofVerifier::new(vk);

        let owner_secret = Fr::from(42u64);
        let input_value = Fr::from(100u64);
        let input_randomness = Fr::from(7u64);
        let out_value = Fr::from(100u64);
        let out_rand = Fr::from(13u64);
        let out_pubkey = Fr::from(99u64);

        let (proof_bytes, mut signals) = prover
            .prove(
                owner_secret,
                input_value,
                input_randomness,
                vec![out_value],
                vec![out_rand],
                vec![out_pubkey],
            )
            .unwrap();

        signals[0] = Fr::from(999u64);
        assert!(verifier.verify(&proof_bytes, &signals).is_err());
    }

    #[test]
    fn insufficient_signals_rejected() {
        let (_pk, vk) = setup_keys();
        let verifier = SpendProofVerifier::new(vk);
        let result = verifier.verify(&[0u8; 128], &[Fr::from(1u64)]);
        assert!(matches!(
            result,
            Err(VerifierError::InsufficientSignals { .. })
        ));
    }

    #[test]
    fn invalid_proof_bytes_rejected() {
        let (_pk, vk) = setup_keys();
        let verifier = SpendProofVerifier::new(vk);
        let signals = vec![Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)];
        let result = verifier.verify(&[0xFF; 10], &signals);
        assert!(matches!(result, Err(VerifierError::InvalidFormat(_))));
    }

    fn setup_keys_n(num_outputs: usize) -> (ark_groth16::ProvingKey<Bn254>, VerifyingKey<Bn254>) {
        let blank = SpendCircuit::blank(num_outputs);
        let mut rng = rand::rngs::OsRng;
        Groth16::<Bn254>::circuit_specific_setup(blank, &mut rng).unwrap()
    }

    #[test]
    fn end_to_end_two_outputs() {
        let (pk, vk) = setup_keys_n(2);
        let prover = SpendProver::new(pk);
        let verifier = SpendProofVerifier::new(vk);

        let secret = Fr::from(5u64);
        let value = Fr::from(200u64);
        let rand_in = Fr::from(3u64);

        let out_v1 = Fr::from(120u64);
        let out_r1 = Fr::from(11u64);
        let out_p1 = Fr::from(77u64);
        let out_v2 = Fr::from(80u64);
        let out_r2 = Fr::from(22u64);
        let out_p2 = Fr::from(88u64);

        let commitment = compute_commitment(value, secret, rand_in);
        let nullifier = compute_nullifier(secret, commitment);

        let (proof_bytes, signals) = prover
            .prove(
                secret,
                value,
                rand_in,
                vec![out_v1, out_v2],
                vec![out_r1, out_r2],
                vec![out_p1, out_p2],
            )
            .unwrap();

        assert_eq!(signals[0], commitment);
        assert_eq!(signals[1], nullifier);
        assert!(verifier.verify(&proof_bytes, &signals).is_ok());
    }
}
