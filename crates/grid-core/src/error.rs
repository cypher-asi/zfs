use std::fmt;
use thiserror::Error;

/// Wire-safe error code for protocol messages.
///
/// A subset of [`GridError`] that can be serialized in protocol responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorCode {
    StorageFull,
    ProofInvalid,
    PolicyReject,
    NotFound,
    InvalidPayload,
    ProgramMismatch,
    SlotOccupied,
    BatchTooLarge,
    ConditionFailed,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StorageFull => f.write_str("storage full"),
            Self::ProofInvalid => f.write_str("proof invalid"),
            Self::PolicyReject => f.write_str("policy reject"),
            Self::NotFound => f.write_str("not found"),
            Self::InvalidPayload => f.write_str("invalid payload"),
            Self::ProgramMismatch => f.write_str("program mismatch"),
            Self::SlotOccupied => f.write_str("slot occupied"),
            Self::BatchTooLarge => f.write_str("batch too large"),
            Self::ConditionFailed => f.write_str("condition failed"),
        }
    }
}

/// Sector-specific store error with structured detail.
#[derive(Debug, Error)]
pub enum SectorStoreError {
    #[error("batch too large: {0}")]
    BatchTooLarge(String),
}

impl From<SectorStoreError> for ErrorCode {
    fn from(e: SectorStoreError) -> Self {
        match e {
            SectorStoreError::BatchTooLarge(_) => Self::BatchTooLarge,
        }
    }
}

/// Shared error type used across Grid crates.
#[derive(Debug, Error)]
pub enum GridError {
    #[error("storage full")]
    StorageFull,
    #[error("proof invalid")]
    ProofInvalid,
    #[error("policy reject")]
    PolicyReject,
    #[error("not found")]
    NotFound,
    #[error("invalid payload: {0}")]
    InvalidPayload(String),
    #[error("program mismatch")]
    ProgramMismatch,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode error: {0}")]
    Encode(String),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("{0}")]
    Other(String),
}

impl From<ErrorCode> for GridError {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::StorageFull => Self::StorageFull,
            ErrorCode::ProofInvalid => Self::ProofInvalid,
            ErrorCode::PolicyReject => Self::PolicyReject,
            ErrorCode::NotFound => Self::NotFound,
            ErrorCode::InvalidPayload => Self::InvalidPayload(String::new()),
            ErrorCode::ProgramMismatch => Self::ProgramMismatch,
            ErrorCode::SlotOccupied => Self::InvalidPayload("slot occupied".into()),
            ErrorCode::BatchTooLarge => Self::InvalidPayload("batch too large".into()),
            ErrorCode::ConditionFailed => Self::InvalidPayload("condition failed".into()),
        }
    }
}

impl GridError {
    /// Convert to a wire-safe error code, if this variant has one.
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            Self::StorageFull => Some(ErrorCode::StorageFull),
            Self::ProofInvalid => Some(ErrorCode::ProofInvalid),
            Self::PolicyReject => Some(ErrorCode::PolicyReject),
            Self::NotFound => Some(ErrorCode::NotFound),
            Self::InvalidPayload(_) => Some(ErrorCode::InvalidPayload),
            Self::ProgramMismatch => Some(ErrorCode::ProgramMismatch),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_storage_full() {
        assert_eq!(
            GridError::StorageFull.error_code(),
            Some(ErrorCode::StorageFull)
        );
    }

    #[test]
    fn error_code_proof_invalid() {
        assert_eq!(
            GridError::ProofInvalid.error_code(),
            Some(ErrorCode::ProofInvalid)
        );
    }

    #[test]
    fn error_code_policy_reject() {
        assert_eq!(
            GridError::PolicyReject.error_code(),
            Some(ErrorCode::PolicyReject)
        );
    }

    #[test]
    fn error_code_not_found() {
        assert_eq!(GridError::NotFound.error_code(), Some(ErrorCode::NotFound));
    }

    #[test]
    fn error_code_invalid_payload() {
        assert_eq!(
            GridError::InvalidPayload("bad data".into()).error_code(),
            Some(ErrorCode::InvalidPayload),
        );
    }

    #[test]
    fn error_code_program_mismatch() {
        assert_eq!(
            GridError::ProgramMismatch.error_code(),
            Some(ErrorCode::ProgramMismatch)
        );
    }

    #[test]
    fn error_code_io_returns_none() {
        let err = GridError::Io(std::io::Error::other("oops"));
        assert_eq!(err.error_code(), None);
    }

    #[test]
    fn error_code_encode_returns_none() {
        assert_eq!(GridError::Encode("fail".into()).error_code(), None);
    }

    #[test]
    fn error_code_decode_returns_none() {
        assert_eq!(GridError::Decode("fail".into()).error_code(), None);
    }

    #[test]
    fn error_code_other_returns_none() {
        assert_eq!(GridError::Other("misc".into()).error_code(), None);
    }

    #[test]
    fn error_code_display_strings() {
        assert_eq!(ErrorCode::StorageFull.to_string(), "storage full");
        assert_eq!(ErrorCode::ProofInvalid.to_string(), "proof invalid");
        assert_eq!(ErrorCode::PolicyReject.to_string(), "policy reject");
        assert_eq!(ErrorCode::NotFound.to_string(), "not found");
        assert_eq!(ErrorCode::InvalidPayload.to_string(), "invalid payload");
        assert_eq!(ErrorCode::ProgramMismatch.to_string(), "program mismatch");
        assert_eq!(ErrorCode::SlotOccupied.to_string(), "slot occupied");
        assert_eq!(ErrorCode::BatchTooLarge.to_string(), "batch too large");
        assert_eq!(ErrorCode::ConditionFailed.to_string(), "condition failed");
    }

    #[test]
    fn error_code_from_sector_store_error() {
        let err = SectorStoreError::BatchTooLarge("too many".into());
        assert_eq!(ErrorCode::from(err), ErrorCode::BatchTooLarge);
    }

    #[test]
    fn grid_error_from_error_code_round_trip() {
        let codes = [
            ErrorCode::StorageFull,
            ErrorCode::ProofInvalid,
            ErrorCode::PolicyReject,
            ErrorCode::NotFound,
            ErrorCode::ProgramMismatch,
        ];
        for code in codes {
            let err = GridError::from(code);
            assert_eq!(err.error_code(), Some(code));
        }
    }

    #[test]
    fn grid_error_from_error_code_slot_occupied_maps_to_invalid_payload() {
        let err = GridError::from(ErrorCode::SlotOccupied);
        assert_eq!(err.error_code(), Some(ErrorCode::InvalidPayload));
    }
}
