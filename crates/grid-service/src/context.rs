use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grid_core::{
    ProgramId, SectorAppendRequest, SectorId, SectorLogLengthRequest, SectorReadLogRequest,
    SectorRequest, SectorResponse,
};
use grid_rpc::SectorDispatch;
use hmac::{Hmac, Mac};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::descriptor::ServiceId;
use crate::error::ServiceError;

type HmacSha256 = Hmac<Sha256>;

/// Events emitted by services for observability.
#[derive(Debug, Clone)]
pub enum ServiceEvent {
    Started {
        service_id: ServiceId,
    },
    Stopped {
        service_id: ServiceId,
    },
    RequestHandled {
        service_id: ServiceId,
        path: String,
        status: u16,
    },
}

/// The service's window into the Zode.
///
/// Provides read/write access to Programs via [`ProgramStore`], helpers for
/// stateless ephemeral flows, event broadcasting, and graceful shutdown.
pub struct ServiceContext {
    pub service_id: ServiceId,
    sector_dispatch: Arc<dyn SectorDispatch>,
    ephemeral_key: [u8; 32],
    pub event_tx: broadcast::Sender<ServiceEvent>,
    pub shutdown: CancellationToken,
}

impl ServiceContext {
    pub fn new(
        service_id: ServiceId,
        sector_dispatch: Arc<dyn SectorDispatch>,
        ephemeral_key: [u8; 32],
        event_tx: broadcast::Sender<ServiceEvent>,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            service_id,
            sector_dispatch,
            ephemeral_key,
            event_tx,
            shutdown,
        }
    }

    /// Get a [`ProgramStore`] for key-value operations on a specific program.
    pub fn store(&self, program_id: &ProgramId) -> ProgramStore {
        ProgramStore {
            program_id: *program_id,
            sector_dispatch: Arc::clone(&self.sector_dispatch),
        }
    }

    /// Create a signed, encrypted, time-limited token from arbitrary payload.
    ///
    /// The client holds this token and presents it back to complete a flow
    /// (auth challenge, OAuth nonce, etc.). This eliminates the need for
    /// server-side ephemeral storage.
    pub fn create_ephemeral_token<T: Serialize>(
        &self,
        payload: &T,
        ttl: Duration,
    ) -> Result<String, ServiceError> {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| ServiceError::EphemeralToken(e.to_string()))?
            .as_secs()
            + ttl.as_secs();

        let wrapper = EphemeralWrapper {
            payload: serde_json::to_value(payload)
                .map_err(|e| ServiceError::EphemeralToken(e.to_string()))?,
            expires_at,
        };

        let json = serde_json::to_vec(&wrapper)
            .map_err(|e| ServiceError::EphemeralToken(e.to_string()))?;

        let mut mac = HmacSha256::new_from_slice(&self.ephemeral_key)
            .map_err(|e| ServiceError::EphemeralToken(e.to_string()))?;
        mac.update(&json);
        let signature = mac.finalize().into_bytes();

        let mut token_bytes = Vec::with_capacity(32 + json.len());
        token_bytes.extend_from_slice(&signature);
        token_bytes.extend_from_slice(&json);

        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &token_bytes,
        ))
    }

    /// Verify and decrypt an ephemeral token, checking expiry.
    pub fn verify_ephemeral_token<T: DeserializeOwned>(
        &self,
        token: &str,
    ) -> Result<T, ServiceError> {
        let token_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            token,
        )
        .map_err(|e| ServiceError::EphemeralToken(format!("invalid base64: {e}")))?;

        if token_bytes.len() < 32 {
            return Err(ServiceError::EphemeralToken("token too short".into()));
        }

        let (sig_bytes, json_bytes) = token_bytes.split_at(32);

        let mut mac = HmacSha256::new_from_slice(&self.ephemeral_key)
            .map_err(|e| ServiceError::EphemeralToken(e.to_string()))?;
        mac.update(json_bytes);
        mac.verify_slice(sig_bytes)
            .map_err(|_| ServiceError::EphemeralToken("invalid signature".into()))?;

        let wrapper: EphemeralWrapper = serde_json::from_slice(json_bytes)
            .map_err(|e| ServiceError::EphemeralToken(format!("invalid payload: {e}")))?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| ServiceError::EphemeralToken(e.to_string()))?
            .as_secs();

        if now > wrapper.expires_at {
            return Err(ServiceError::EphemeralToken("token expired".into()));
        }

        serde_json::from_value(wrapper.payload)
            .map_err(|e| ServiceError::EphemeralToken(format!("payload deserialize: {e}")))
    }
}

#[derive(Serialize, serde::Deserialize)]
struct EphemeralWrapper {
    payload: serde_json::Value,
    expires_at: u64,
}

/// Key-value abstraction over sector append-logs.
///
/// Maps familiar get/put/list semantics onto the Grid's append-only sector
/// storage. This is the critical abstraction that lets services replace their
/// own databases (e.g. RocksDB) with Grid Programs.
///
/// - **Key** maps to a `SectorId` via `SHA-256(key)`
/// - **Put** appends a new entry to that sector (latest entry = current value)
/// - **Get** reads the last entry from the sector log
/// - **List** reads the full sector log (all historical values)
pub struct ProgramStore {
    program_id: ProgramId,
    sector_dispatch: Arc<dyn SectorDispatch>,
}

impl ProgramStore {
    /// Read the latest value for a key.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        let len = self.sector_length(&sector_id)?;
        if len == 0 {
            return Ok(None);
        }
        let entries = self.read_log(&sector_id, len - 1, 1)?;
        Ok(entries.into_iter().next())
    }

    /// Write a value for a key (appends to sector log; latest = current).
    pub fn put(&self, key: &[u8], value: Vec<u8>) -> Result<(), ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        let req = SectorRequest::Append(SectorAppendRequest {
            program_id: self.program_id,
            sector_id,
            entry: value,
            shape_proof: None,
        });
        let resp = self.sector_dispatch.dispatch(&req);
        match resp {
            SectorResponse::Append(r) if r.ok => Ok(()),
            SectorResponse::Append(r) => Err(ServiceError::Storage(format!(
                "append failed: {:?}",
                r.error_code
            ))),
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Read all entries for a key (full history / collection).
    pub fn list(&self, key: &[u8]) -> Result<Vec<Vec<u8>>, ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        self.read_log(&sector_id, 0, u32::MAX)
    }

    /// Read entries for a key starting from an index.
    pub fn list_from(&self, key: &[u8], from: u64) -> Result<Vec<Vec<u8>>, ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        self.read_log(&sector_id, from, u32::MAX)
    }

    /// Get the number of entries for a key.
    pub fn len(&self, key: &[u8]) -> Result<u64, ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        self.sector_length(&sector_id)
    }

    /// Check if a key has any entries.
    pub fn is_empty(&self, key: &[u8]) -> Result<bool, ServiceError> {
        Ok(self.len(key)? == 0)
    }

    /// Raw sector dispatch for advanced use cases.
    pub fn raw(&self) -> &Arc<dyn SectorDispatch> {
        &self.sector_dispatch
    }

    /// The program ID this store operates on.
    pub fn program_id(&self) -> &ProgramId {
        &self.program_id
    }

    fn key_to_sector_id(&self, key: &[u8]) -> SectorId {
        let hash = Sha256::digest(key);
        SectorId::from_bytes(hash.to_vec())
    }

    fn sector_length(&self, sector_id: &SectorId) -> Result<u64, ServiceError> {
        let req = SectorRequest::LogLength(SectorLogLengthRequest {
            program_id: self.program_id,
            sector_id: sector_id.clone(),
        });
        let resp = self.sector_dispatch.dispatch(&req);
        match resp {
            SectorResponse::LogLength(r) => {
                if let Some(code) = r.error_code {
                    Err(ServiceError::Storage(format!("log length error: {code}")))
                } else {
                    Ok(r.length)
                }
            }
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    fn read_log(
        &self,
        sector_id: &SectorId,
        from: u64,
        max: u32,
    ) -> Result<Vec<Vec<u8>>, ServiceError> {
        let req = SectorRequest::ReadLog(SectorReadLogRequest {
            program_id: self.program_id,
            sector_id: sector_id.clone(),
            from_index: from,
            max_entries: max,
        });
        let resp = self.sector_dispatch.dispatch(&req);
        match resp {
            SectorResponse::ReadLog(r) => {
                if let Some(code) = r.error_code {
                    Err(ServiceError::Storage(format!("read log error: {code}")))
                } else {
                    Ok(r.entries.into_iter().map(|b| b.into_vec()).collect())
                }
            }
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }
}
