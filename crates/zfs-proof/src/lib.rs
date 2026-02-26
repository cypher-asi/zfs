#![forbid(unsafe_code)]
//! Pluggable Valid-Sector proof verification for ZFS.
//!
//! Provides the [`ProofVerifier`] trait that Zodes use to verify incoming
//! sector proofs, plus the [`NoopVerifier`] default for v0.1.0.
//!
//! # Proof semantics
//!
//! Proofs attest that encrypted sector content conforms to a program's
//! field schema without revealing the plaintext. Verification is bound to
//! `(cid, program_id, version)` and uses per-program verifier keys.
//!
//! # Verifier key storage
//!
//! Verifier keys are loaded by the [`ProofVerifier`] implementation from a
//! local program store. The store location is implementation-defined and
//! configured via `ZodeConfig` (see `zfs-zode`).

mod error;
mod noop;
mod verifier;

pub use error::ProofError;
pub use noop::NoopVerifier;
pub use verifier::{ProofVerifier, ProofVerifierRegistry, VerifiedSector};

#[cfg(test)]
mod tests {
    use super::*;
    use zfs_core::{Cid, ProgramId};

    fn test_cid() -> Cid {
        Cid::from_ciphertext(b"test ciphertext")
    }

    fn test_program_id() -> ProgramId {
        ProgramId::from_descriptor_bytes(b"test program descriptor")
    }

    #[test]
    fn noop_verifier_always_succeeds() {
        let verifier = NoopVerifier;
        let cid = test_cid();
        let pid = test_program_id();

        let verified = verifier.verify(&cid, &pid, 1, b"any proof", None).unwrap();
        assert_eq!(verified.cid, cid);
        assert_eq!(verified.program_id, pid);
        assert_eq!(verified.version, 1);
    }

    #[test]
    fn noop_verifier_succeeds_with_empty_proof() {
        let verifier = NoopVerifier;
        let result = verifier.verify(&test_cid(), &test_program_id(), 0, b"", None);
        assert!(result.is_ok());
    }

    #[test]
    fn noop_verifier_succeeds_with_payload_context() {
        let verifier = NoopVerifier;
        let result = verifier.verify(
            &test_cid(),
            &test_program_id(),
            42,
            b"proof bytes",
            Some(b"context bytes"),
        );
        let verified = result.unwrap();
        assert_eq!(verified.version, 42);
    }

    #[test]
    fn proof_error_verifier_key_not_found() {
        let err = ProofError::VerifierKeyNotFound {
            program_id: "abc123".into(),
        };
        assert!(err.to_string().contains("abc123"));
    }

    #[test]
    fn proof_error_verification_failed() {
        let err = ProofError::VerificationFailed {
            reason: "invalid circuit".into(),
        };
        assert!(err.to_string().contains("invalid circuit"));
    }

    #[test]
    fn proof_error_invalid_format() {
        let err = ProofError::InvalidProofFormat {
            reason: "truncated".into(),
        };
        assert!(err.to_string().contains("truncated"));
    }

    #[test]
    fn verified_sector_equality() {
        let cid = test_cid();
        let pid = test_program_id();

        let a = VerifiedSector {
            cid,
            program_id: pid,
            version: 1,
        };
        let b = VerifiedSector {
            cid,
            program_id: pid,
            version: 1,
        };
        assert_eq!(a, b);

        let c = VerifiedSector {
            cid,
            program_id: pid,
            version: 2,
        };
        assert_ne!(a, c);
    }

    #[test]
    fn proof_verifier_is_object_safe() {
        let verifier: Box<dyn ProofVerifier> = Box::new(NoopVerifier);
        let result = verifier.verify(&test_cid(), &test_program_id(), 1, b"", None);
        assert!(result.is_ok());
    }

    #[test]
    fn proof_verifier_registry_dispatch() {
        use std::sync::Arc;
        use zfs_core::ProofSystem;

        let mut registry = ProofVerifierRegistry::new();
        registry.register(ProofSystem::None, Arc::new(NoopVerifier));

        assert!(registry.has_verifier(&ProofSystem::None));

        let result = registry.verify(
            &ProofSystem::None,
            &test_cid(),
            &test_program_id(),
            1,
            b"proof",
            None,
        );
        let verified = result.expect("registry dispatch should succeed");
        assert_eq!(verified.cid, test_cid());
        assert_eq!(verified.program_id, test_program_id());
    }

    #[test]
    fn proof_verifier_registry_missing_system() {
        use zfs_core::ProofSystem;

        let registry = ProofVerifierRegistry::new();
        assert!(!registry.has_verifier(&ProofSystem::Groth16));

        let result = registry.verify(
            &ProofSystem::Groth16,
            &test_cid(),
            &test_program_id(),
            1,
            b"proof",
            None,
        );
        assert!(result.is_err());
    }
}
