use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use grid_core::{
    KvContainsRequest, KvDeleteRequest, KvGetRequest, KvPutRequest, ProgramId, SectorAppendRequest,
    SectorId, SectorLogLengthRequest, SectorReadLogRequest, SectorRequest, SectorResponse,
};
use grid_rpc::SectorDispatch;
use hmac::{Hmac, Mac};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::descriptor::ServiceId;
use crate::error::ServiceError;
use crate::identity::NodeIdentity;

/// Command to dynamically manage GossipSub subscriptions at runtime.
#[derive(Debug, Clone)]
pub enum TopicCommand {
    Subscribe(String),
    Unsubscribe(String),
}

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
    publish_tx: Option<mpsc::Sender<(String, Vec<u8>)>>,
    topic_tx: Option<mpsc::Sender<TopicCommand>>,
    direct_tx: Option<mpsc::Sender<(String, String, Vec<u8>)>>,
    identity: Option<Arc<NodeIdentity>>,
    proof_registry: Option<Arc<grid_proof::ProofVerifierRegistry>>,
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
            publish_tx: None,
            topic_tx: None,
            direct_tx: None,
            identity: None,
            proof_registry: None,
        }
    }

    /// Access the node's identity for signing and identification.
    pub fn identity(&self) -> Option<&NodeIdentity> {
        self.identity.as_deref()
    }

    /// Set the node identity. Called during startup.
    pub fn set_identity(&mut self, identity: Arc<NodeIdentity>) {
        self.identity = Some(identity);
    }

    /// Access the proof verifier registry for standalone proof verification.
    ///
    /// Returns `None` if the Zode was started without proof verification
    /// support (e.g. in test configurations).
    pub fn proof_registry(&self) -> Option<&Arc<grid_proof::ProofVerifierRegistry>> {
        self.proof_registry.as_ref()
    }

    /// Set the proof verifier registry. Called during startup.
    pub fn set_proof_registry(&mut self, registry: Arc<grid_proof::ProofVerifierRegistry>) {
        self.proof_registry = Some(registry);
    }

    /// Set the GossipSub publish and topic management channels.
    ///
    /// Called by `ServiceRegistry` during startup when the Zode provides
    /// its channel endpoints.
    pub fn set_channels(
        &mut self,
        publish_tx: mpsc::Sender<(String, Vec<u8>)>,
        topic_tx: mpsc::Sender<TopicCommand>,
    ) {
        self.publish_tx = Some(publish_tx);
        self.topic_tx = Some(topic_tx);
    }

    /// Set the direct-message outbound channel.
    pub fn set_direct_channel(&mut self, direct_tx: mpsc::Sender<(String, String, Vec<u8>)>) {
        self.direct_tx = Some(direct_tx);
    }

    /// Get a clone of the publish channel sender for use in spawned tasks.
    pub fn publish_sender(&self) -> Option<mpsc::Sender<(String, Vec<u8>)>> {
        self.publish_tx.clone()
    }

    /// Get a clone of the topic management channel sender for use in spawned tasks.
    pub fn topic_sender(&self) -> Option<mpsc::Sender<TopicCommand>> {
        self.topic_tx.clone()
    }

    /// Publish a message to a GossipSub topic.
    ///
    /// Non-blocking; queues the message for the Zode event loop to send.
    pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), ServiceError> {
        let tx = self
            .publish_tx
            .as_ref()
            .ok_or_else(|| ServiceError::NotInitialized("publish channel not set".into()))?;
        tx.try_send((topic.to_owned(), data))
            .map_err(|e| ServiceError::Other(format!("publish channel: {e}")))
    }

    /// Subscribe to a GossipSub topic at runtime.
    pub fn subscribe_topic(&self, topic: &str) -> Result<(), ServiceError> {
        let tx = self
            .topic_tx
            .as_ref()
            .ok_or_else(|| ServiceError::NotInitialized("topic channel not set".into()))?;
        tx.try_send(TopicCommand::Subscribe(topic.to_owned()))
            .map_err(|e| ServiceError::Other(format!("topic channel: {e}")))
    }

    /// Unsubscribe from a GossipSub topic at runtime.
    pub fn unsubscribe_topic(&self, topic: &str) -> Result<(), ServiceError> {
        let tx = self
            .topic_tx
            .as_ref()
            .ok_or_else(|| ServiceError::NotInitialized("topic channel not set".into()))?;
        tx.try_send(TopicCommand::Unsubscribe(topic.to_owned()))
            .map_err(|e| ServiceError::Other(format!("topic channel: {e}")))
    }

    /// Send a direct (point-to-point) message to another Zode.
    ///
    /// `target` is the `ZodeId` string of the destination peer.
    pub fn send_direct(
        &self,
        target: &str,
        topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), ServiceError> {
        let tx = self
            .direct_tx
            .as_ref()
            .ok_or_else(|| ServiceError::NotInitialized("direct channel not set".into()))?;
        tx.try_send((target.to_owned(), topic.to_owned(), payload))
            .map_err(|e| ServiceError::Other(format!("direct channel: {e}")))
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
        let token_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, token)
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

    /// Default pagination limit for unbounded reads.
    const DEFAULT_PAGE_LIMIT: u32 = 10_000;

    /// Read all entries for a key (full history / collection), up to the
    /// default pagination limit.
    pub fn list(&self, key: &[u8]) -> Result<Vec<Vec<u8>>, ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        self.read_log(&sector_id, 0, Self::DEFAULT_PAGE_LIMIT)
    }

    /// Read entries for a key starting from an index, up to the default
    /// pagination limit.
    pub fn list_from(&self, key: &[u8], from: u64) -> Result<Vec<Vec<u8>>, ServiceError> {
        let sector_id = self.key_to_sector_id(key);
        self.read_log(&sector_id, from, Self::DEFAULT_PAGE_LIMIT)
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

    /// Get a value from the service KV store (local index, not append-log).
    pub fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> {
        let req = SectorRequest::KvGet(KvGetRequest {
            program_id: self.program_id,
            key: key.to_vec(),
        });
        match self.sector_dispatch.dispatch(&req) {
            SectorResponse::KvGet(r) => {
                if let Some(code) = r.error_code {
                    Err(ServiceError::Storage(format!("kv_get error: {code}")))
                } else {
                    Ok(r.value)
                }
            }
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Put a value into the service KV store (local index, not append-log).
    pub fn kv_put(&self, key: &[u8], value: Vec<u8>) -> Result<(), ServiceError> {
        let req = SectorRequest::KvPut(KvPutRequest {
            program_id: self.program_id,
            key: key.to_vec(),
            value,
        });
        match self.sector_dispatch.dispatch(&req) {
            SectorResponse::KvPut(r) if r.ok => Ok(()),
            SectorResponse::KvPut(r) => Err(ServiceError::Storage(format!(
                "kv_put failed: {:?}",
                r.error_code
            ))),
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Delete a key from the service KV store.
    pub fn kv_delete(&self, key: &[u8]) -> Result<(), ServiceError> {
        let req = SectorRequest::KvDelete(KvDeleteRequest {
            program_id: self.program_id,
            key: key.to_vec(),
        });
        match self.sector_dispatch.dispatch(&req) {
            SectorResponse::KvDelete(r) if r.ok => Ok(()),
            SectorResponse::KvDelete(r) => Err(ServiceError::Storage(format!(
                "kv_delete failed: {:?}",
                r.error_code
            ))),
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
    }

    /// Check if a key exists in the service KV store.
    pub fn kv_contains(&self, key: &[u8]) -> Result<bool, ServiceError> {
        let req = SectorRequest::KvContains(KvContainsRequest {
            program_id: self.program_id,
            key: key.to_vec(),
        });
        match self.sector_dispatch.dispatch(&req) {
            SectorResponse::KvContains(r) => {
                if let Some(code) = r.error_code {
                    Err(ServiceError::Storage(format!("kv_contains error: {code}")))
                } else {
                    Ok(r.exists)
                }
            }
            other => Err(ServiceError::Storage(format!(
                "unexpected response: {other:?}"
            ))),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use grid_core::{SectorRequest, SectorResponse};

    struct StubDispatch;
    impl grid_rpc::SectorDispatch for StubDispatch {
        fn dispatch(&self, _req: &SectorRequest) -> SectorResponse {
            unimplemented!("stub")
        }
    }

    fn make_context() -> ServiceContext {
        let (event_tx, _) = broadcast::channel(16);
        ServiceContext::new(
            crate::descriptor::ServiceId::from([0u8; 32]),
            Arc::new(StubDispatch),
            [0u8; 32],
            event_tx,
            CancellationToken::new(),
        )
    }

    #[test]
    fn publish_without_channel_returns_not_initialized() {
        let ctx = make_context();
        let err = ctx.publish("topic", b"data".to_vec()).unwrap_err();
        assert!(matches!(err, ServiceError::NotInitialized(_)));
    }

    #[test]
    fn publish_queues_message_on_channel() {
        let mut ctx = make_context();
        let (pub_tx, mut pub_rx) = mpsc::channel(8);
        let (topic_tx, _topic_rx) = mpsc::channel(8);
        ctx.set_channels(pub_tx, topic_tx);

        ctx.publish("my-topic", b"hello".to_vec()).unwrap();

        let (topic, data) = pub_rx.try_recv().unwrap();
        assert_eq!(topic, "my-topic");
        assert_eq!(data, b"hello");
    }

    #[test]
    fn subscribe_topic_sends_command() {
        let mut ctx = make_context();
        let (pub_tx, _pub_rx) = mpsc::channel(8);
        let (topic_tx, mut topic_rx) = mpsc::channel(8);
        ctx.set_channels(pub_tx, topic_tx);

        ctx.subscribe_topic("consensus/epoch-1").unwrap();

        match topic_rx.try_recv().unwrap() {
            TopicCommand::Subscribe(t) => assert_eq!(t, "consensus/epoch-1"),
            other => panic!("expected Subscribe, got {other:?}"),
        }
    }

    #[test]
    fn unsubscribe_topic_sends_command() {
        let mut ctx = make_context();
        let (pub_tx, _pub_rx) = mpsc::channel(8);
        let (topic_tx, mut topic_rx) = mpsc::channel(8);
        ctx.set_channels(pub_tx, topic_tx);

        ctx.unsubscribe_topic("old-topic").unwrap();

        match topic_rx.try_recv().unwrap() {
            TopicCommand::Unsubscribe(t) => assert_eq!(t, "old-topic"),
            other => panic!("expected Unsubscribe, got {other:?}"),
        }
    }

    #[test]
    fn send_direct_without_channel_returns_not_initialized() {
        let ctx = make_context();
        let err = ctx
            .send_direct("Zxpeer1", "topic", b"hi".to_vec())
            .unwrap_err();
        assert!(matches!(err, ServiceError::NotInitialized(_)));
    }

    #[test]
    fn send_direct_queues_message_on_channel() {
        let mut ctx = make_context();
        let (dtx, mut drx) = mpsc::channel(8);
        ctx.set_direct_channel(dtx);

        ctx.send_direct("Zxpeer1", "my-topic", b"hello".to_vec())
            .unwrap();

        let (target, topic, payload) = drx.try_recv().unwrap();
        assert_eq!(target, "Zxpeer1");
        assert_eq!(topic, "my-topic");
        assert_eq!(payload, b"hello");
    }
}
