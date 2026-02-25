use std::fmt;
use thiserror::Error;

/// Wire-safe error code for protocol messages.
///
/// A subset of [`ZfsError`] that can be serialized in protocol responses.
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
    #[error("slot occupied: sector already has a value")]
    SlotOccupied,
    #[error("condition failed: expected hash mismatch")]
    ConditionFailed,
    #[error("batch too large: {0}")]
    BatchTooLarge(String),
}

impl From<SectorStoreError> for ErrorCode {
    fn from(e: SectorStoreError) -> Self {
        match e {
            SectorStoreError::SlotOccupied => Self::SlotOccupied,
            SectorStoreError::ConditionFailed => Self::ConditionFailed,
            SectorStoreError::BatchTooLarge(_) => Self::BatchTooLarge,
        }
    }
}

/// Shared error type used across ZFS crates.
#[derive(Debug, Error)]
pub enum ZfsError {
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

impl From<ErrorCode> for ZfsError {
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

impl ZfsError {
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
