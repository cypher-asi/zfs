#![allow(dead_code)]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("service not found: {0}")]
    NotFound(String),

    #[error("service already registered: {0}")]
    AlreadyRegistered(String),

    #[error("missing required program: {0}")]
    MissingProgram(String),

    #[error("descriptor error: {0}")]
    Descriptor(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("ephemeral token error: {0}")]
    EphemeralToken(String),

    #[error("startup failed: {0}")]
    Startup(String),

    #[error("shutdown failed: {0}")]
    Shutdown(String),

    #[error("grid error: {0}")]
    Grid(#[from] grid_core::GridError),

    #[error("{0}")]
    Other(String),
}
