use thiserror::Error;

/// Errors produced by proof verification.
#[derive(Debug, Error)]
pub enum ProofError {
    #[error("no verifier registered for proof system {proof_system}")]
    VerifierNotFound { proof_system: String },

    #[error("proof verification failed: {reason}")]
    VerificationFailed { reason: String },

    #[error("invalid proof format: {reason}")]
    InvalidProofFormat { reason: String },
}
