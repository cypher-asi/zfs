# GRID Protocol Extensions — Service-Layer Primitives

This document specifies six additions to the GRID core protocol that give
services first-class access to gossip, storage, networking, identity, and proof
verification.  Every change is backwards-compatible; existing services
(Identity, Interlink) continue to work unmodified.

The changes are ordered by priority.  P0 items should land first since all
subsequent work (including any consensus-based service) depends on them.

---

## Table of Contents

1. [ServiceGossipHandler (P0)](#1-servicegossiphandler)
2. [Publish / Subscribe in ServiceContext (P0)](#2-publish--subscribe-in-servicecontext)
3. [Key-Value Storage in ProgramStore (P1)](#3-key-value-storage-in-programstore)
4. [Point-to-Point Messaging (P1)](#4-point-to-point-messaging)
5. [Node Identity in ServiceContext (P2)](#5-node-identity-in-servicecontext)
6. [Proof Registry in ServiceContext (P2)](#6-proof-registry-in-servicecontext)

---

## 1. ServiceGossipHandler

**Priority:** P0
**Crates touched:** `grid-service`, `zode`

### Problem

All GossipSub messages flow through a single handler in
`crates/zode/src/gossip.rs:9-39` (`handle_gossip_message`) which hard-codes
`GossipSectorAppend` as the only valid payload.  Services that define their own
wire protocols (consensus messages, coordination signals, etc.) cannot receive
gossip on their owned program topics.

### Design

Add a trait to `grid-service` that services optionally implement.  The Zode
checks registered handlers **before** falling through to the default
`GossipSectorAppend` path.

### Implementation

#### 1a. Define the trait — `crates/grid-service/src/gossip.rs` (new file)

```rust
use async_trait::async_trait;

/// Allows a service to intercept GossipSub messages on topics it owns.
///
/// The Zode dispatches incoming gossip to registered handlers before
/// falling through to the default `GossipSectorAppend` path.  A handler
/// that returns `true` from `handles_topic` receives the raw bytes; the
/// default handler is skipped for that message.
#[async_trait]
pub trait ServiceGossipHandler: Send + Sync + 'static {
    /// Return `true` if this handler should receive messages on `topic`.
    fn handles_topic(&self, topic: &str) -> bool;

    /// Called for every gossip message on a handled topic.
    ///
    /// `data` is the raw CBOR payload from GossipSub.
    /// `sender` is the formatted ZodeId of the message source, if known.
    async fn on_gossip(&self, topic: &str, data: &[u8], sender: Option<String>);
}
```

#### 1b. Re-export from lib.rs — `crates/grid-service/src/lib.rs`

Add to existing exports:

```rust
mod gossip;
pub use gossip::ServiceGossipHandler;
```

#### 1c. Store handlers in ServiceRegistry — `crates/grid-service/src/registry.rs`

Add a field to `ServiceRegistry`:

```rust
pub struct ServiceRegistry {
    // ... existing fields ...
    gossip_handlers: Vec<Arc<dyn ServiceGossipHandler>>,
}
```

Add a registration method:

```rust
impl ServiceRegistry {
    pub fn register_gossip_handler(&mut self, handler: Arc<dyn ServiceGossipHandler>) {
        self.gossip_handlers.push(handler);
    }

    /// Find the first handler that claims this topic, if any.
    pub fn gossip_handler_for(&self, topic: &str) -> Option<&Arc<dyn ServiceGossipHandler>> {
        self.gossip_handlers.iter().find(|h| h.handles_topic(topic))
    }
}
```

#### 1d. Dispatch in Zode gossip path — `crates/zode/src/gossip.rs`

Replace the current `handle_gossip_message` with a two-phase dispatch:

```rust
pub(crate) async fn handle_gossip_message<S: SectorStore>(
    sector_handler: &SectorRequestHandler<S>,
    service_registry: &ServiceRegistry,
    event_tx: &broadcast::Sender<LogEvent>,
    topic: &str,
    data: &[u8],
    sender: Option<String>,
) {
    // Phase 1: check if a service handler claims this topic.
    if let Some(handler) = service_registry.gossip_handler_for(topic) {
        handler.on_gossip(topic, data, sender).await;
        return;
    }

    // Phase 2: default GossipSectorAppend path (existing logic).
    match grid_core::decode_canonical::<GossipSectorAppend>(data) {
        Ok(msg) => {
            let result = sector_handler.handle_gossip_append(&msg);
            // ... existing logging and event emission ...
        }
        Err(e) => {
            warn!(%topic, error = %e, "failed to decode gossip message");
        }
    }
}
```

This changes the function signature from sync to async.  The call site in
`Zode::dispatch_event` (`crates/zode/src/zode.rs`, around line 679-691) must
be updated to `.await` the call and pass a reference to the `service_registry`.

#### 1e. Wire into Zode event loop — `crates/zode/src/zode.rs`

The `dispatch_event` method currently calls the gossip handler synchronously in
the `GossipMessage` arm.  Two changes are needed:

1. Pass `&service_registry` (or `&ServiceRegistry`) through to `dispatch_event`.
   The registry is already held as `Mutex<ServiceRegistry>` on the `Zode` struct.
   Take a read lock before entering the event loop iteration, or pass it as a
   parameter.

2. Change the `GossipMessage` handler from:
   ```rust
   crate::gossip::handle_gossip_message(sector_handler, event_tx, &topic, &data, sender);
   ```
   to:
   ```rust
   crate::gossip::handle_gossip_message(
       sector_handler, &registry, event_tx, &topic, &data, sender,
   ).await;
   ```

### Tests

- Unit test: register a handler, verify it receives messages on its topic.
- Unit test: messages on unhandled topics still reach the default
  `GossipSectorAppend` path.
- Unit test: handler returning `handles_topic = false` does not intercept.

---

## 2. Publish / Subscribe in ServiceContext

**Priority:** P0
**Crates touched:** `grid-service`, `zode`

### Problem

`ServiceContext` (`crates/grid-service/src/context.rs:40-46`) provides storage
access and ephemeral tokens but no way to publish GossipSub messages or
dynamically subscribe/unsubscribe from topics at runtime.  Services that need
to broadcast messages or react to changing topic sets (e.g. epoch-based
rotation) cannot do so.

### Design

Plumb the existing `publish_tx` channel from the Zode into `ServiceContext`.
Add a new `topic_tx` channel for subscribe/unsubscribe commands that the Zode
event loop processes.

### Implementation

#### 2a. Define TopicCommand — `crates/grid-service/src/context.rs`

```rust
/// Command to dynamically manage GossipSub subscriptions at runtime.
#[derive(Debug, Clone)]
pub enum TopicCommand {
    Subscribe(String),
    Unsubscribe(String),
}
```

#### 2b. Extend ServiceContext — `crates/grid-service/src/context.rs`

Add two channel senders to the struct and constructor:

```rust
pub struct ServiceContext {
    pub service_id: ServiceId,
    sector_dispatch: Arc<dyn SectorDispatch>,
    ephemeral_key: [u8; 32],
    pub event_tx: broadcast::Sender<ServiceEvent>,
    pub shutdown: CancellationToken,
    publish_tx: mpsc::Sender<(String, Vec<u8>)>,
    topic_tx: mpsc::Sender<TopicCommand>,
}
```

Update `ServiceContext::new` to accept the two new senders.

Add methods:

```rust
impl ServiceContext {
    /// Publish a message to a GossipSub topic.
    ///
    /// Non-blocking; queues the message for the Zode event loop to send.
    pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), ServiceError> {
        self.publish_tx
            .try_send((topic.to_owned(), data))
            .map_err(|e| ServiceError::Other(format!("publish channel: {e}")))
    }

    /// Subscribe to a GossipSub topic at runtime.
    pub fn subscribe_topic(&self, topic: &str) -> Result<(), ServiceError> {
        self.topic_tx
            .try_send(TopicCommand::Subscribe(topic.to_owned()))
            .map_err(|e| ServiceError::Other(format!("topic channel: {e}")))
    }

    /// Unsubscribe from a GossipSub topic at runtime.
    pub fn unsubscribe_topic(&self, topic: &str) -> Result<(), ServiceError> {
        self.topic_tx
            .try_send(TopicCommand::Unsubscribe(topic.to_owned()))
            .map_err(|e| ServiceError::Other(format!("topic channel: {e}")))
    }
}
```

#### 2c. Create channels in ServiceRegistry — `crates/grid-service/src/registry.rs`

`ServiceRegistry::start_all` (and `start_service`) currently builds
`ServiceContext` instances.  They must now also receive `publish_tx` and
`topic_tx`.  Add these as fields on `ServiceRegistry`:

```rust
pub struct ServiceRegistry {
    // ... existing fields ...
    publish_tx: Option<mpsc::Sender<(String, Vec<u8>)>>,
    topic_tx: Option<mpsc::Sender<TopicCommand>>,
}
```

Add a setter called before `start_all`:

```rust
impl ServiceRegistry {
    pub fn set_channels(
        &mut self,
        publish_tx: mpsc::Sender<(String, Vec<u8>)>,
        topic_tx: mpsc::Sender<TopicCommand>,
    ) {
        self.publish_tx = Some(publish_tx);
        self.topic_tx = Some(topic_tx);
    }
}
```

Clone these senders into each `ServiceContext` during `start_all` / `start_service`.

#### 2d. Process TopicCommands in Zode event loop — `crates/zode/src/zode.rs`

Create the `topic_tx` / `topic_rx` channel pair in `Zode::start`, pass `topic_tx`
to the registry via `set_channels`, and add `topic_rx` as a new arm in the
event loop's `tokio::select!`:

```rust
Some(cmd) = topic_rx.recv() => {
    match cmd {
        TopicCommand::Subscribe(topic) => {
            if let Err(e) = net.subscribe(&topic) {
                warn!(error = %e, %topic, "dynamic subscribe failed");
            } else {
                info!(%topic, "dynamic subscribe");
            }
        }
        TopicCommand::Unsubscribe(topic) => {
            if let Err(e) = net.unsubscribe(&topic) {
                warn!(error = %e, %topic, "dynamic unsubscribe failed");
            } else {
                info!(%topic, "dynamic unsubscribe");
            }
        }
    }
    continue;
}
```

The `publish_tx` channel already exists on the Zode (`self.publish_tx`).
Pass a clone of it to the registry alongside `topic_tx`.

#### 2e. Update existing callers

`ServiceContext::new` gains two new parameters.  All call sites in
`ServiceRegistry` must be updated.  The existing `IdentityService` and
`InterlinkService` do not call publish/subscribe and require no changes
beyond accepting the new `ServiceContext` shape.

### Tests

- Unit test: publish queues a message that appears on the `publish_rx`.
- Unit test: `subscribe_topic` / `unsubscribe_topic` produce the correct
  `TopicCommand` on the channel.
- Integration test: a service publishes a gossip message that another peer
  receives.

---

## 3. Key-Value Storage in ProgramStore

**Priority:** P1
**Crates touched:** `grid-storage`, `grid-service`

### Problem

`ProgramStore` (`crates/grid-service/src/context.rs:167-295`) is backed by
append-only sector logs.  Services that need fast key-value lookups (set
membership, counters, mutable state) must rebuild in-memory structures from
the full log on every startup.  This does not scale.

### Design

Add a `service_kv` column family to RocksDB and expose get/put/delete/contains
through `ProgramStore`.  The append-only log remains the source of truth for
replication; the KV store is a local index.

### Implementation

#### 3a. Add column family — `crates/grid-storage/src/rocks.rs`

```rust
pub(crate) const CF_SERVICE_KV: &str = "service_kv";
```

Add it to the `cf_names` array in `RocksStorage::open`:

```rust
let cf_names = [CF_METADATA, CF_SECTORS, CF_PROOFS, CF_SERVICE_KV];
```

#### 3b. Add KV methods to SectorStore trait — `crates/grid-storage/src/sector_traits.rs`

Append to the existing `SectorStore` trait:

```rust
pub trait SectorStore {
    // ... existing methods ...

    /// Get a value by key from the service KV store.
    /// Key layout: `program_id(32B) || key`.
    fn kv_get(
        &self,
        program_id: &ProgramId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError>;

    /// Put a value by key into the service KV store.
    fn kv_put(
        &self,
        program_id: &ProgramId,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StorageError>;

    /// Delete a key from the service KV store.
    fn kv_delete(
        &self,
        program_id: &ProgramId,
        key: &[u8],
    ) -> Result<(), StorageError>;

    /// Check if a key exists in the service KV store.
    fn kv_contains(
        &self,
        program_id: &ProgramId,
        key: &[u8],
    ) -> Result<bool, StorageError>;
}
```

#### 3c. Implement for RocksStorage — `crates/grid-storage/src/sector_rocks.rs`

Key layout: `program_id(32 bytes) || key (variable)`.

```rust
impl SectorStore for RocksStorage {
    // ... existing methods ...

    fn kv_get(&self, program_id: &ProgramId, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        let cf = self.cf_handle(CF_SERVICE_KV)?;
        let db_key = kv_key(program_id, key);
        Ok(self.db().get_cf(cf, &db_key)?)
    }

    fn kv_put(&self, program_id: &ProgramId, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        let cf = self.cf_handle(CF_SERVICE_KV)?;
        let db_key = kv_key(program_id, key);
        self.db().put_cf(cf, &db_key, value)?;
        Ok(())
    }

    fn kv_delete(&self, program_id: &ProgramId, key: &[u8]) -> Result<(), StorageError> {
        let cf = self.cf_handle(CF_SERVICE_KV)?;
        let db_key = kv_key(program_id, key);
        self.db().delete_cf(cf, &db_key)?;
        Ok(())
    }

    fn kv_contains(&self, program_id: &ProgramId, key: &[u8]) -> Result<bool, StorageError> {
        Ok(self.kv_get(program_id, key)?.is_some())
    }
}

fn kv_key(program_id: &ProgramId, key: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + key.len());
    buf.extend_from_slice(program_id.as_bytes());
    buf.extend_from_slice(key);
    buf
}
```

#### 3d. Route through SectorDispatch — `crates/grid-rpc/src/lib.rs` and `crates/grid-core/src/sector_protocol.rs`

Add new variants to `SectorRequest` and `SectorResponse`:

```rust
// In SectorRequest
KvGet(KvGetRequest),
KvPut(KvPutRequest),
KvDelete(KvDeleteRequest),
KvContains(KvContainsRequest),

// New request types
pub struct KvGetRequest { pub program_id: ProgramId, pub key: Vec<u8> }
pub struct KvPutRequest { pub program_id: ProgramId, pub key: Vec<u8>, pub value: Vec<u8> }
pub struct KvDeleteRequest { pub program_id: ProgramId, pub key: Vec<u8> }
pub struct KvContainsRequest { pub program_id: ProgramId, pub key: Vec<u8> }

// In SectorResponse
KvGet(KvGetResponse),
KvPut(KvPutResponse),
KvDelete(KvDeleteResponse),
KvContains(KvContainsResponse),

// New response types
pub struct KvGetResponse { pub value: Option<Vec<u8>>, pub error_code: Option<ErrorCode> }
pub struct KvPutResponse { pub ok: bool, pub error_code: Option<ErrorCode> }
pub struct KvDeleteResponse { pub ok: bool, pub error_code: Option<ErrorCode> }
pub struct KvContainsResponse { pub exists: bool, pub error_code: Option<ErrorCode> }
```

Handle the new variants in `SectorRequestHandler::handle_sector_request`
(`crates/zode/src/sector_handler.rs`).

#### 3e. Expose on ProgramStore — `crates/grid-service/src/context.rs`

```rust
impl ProgramStore {
    pub fn kv_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ServiceError> { ... }
    pub fn kv_put(&self, key: &[u8], value: Vec<u8>) -> Result<(), ServiceError> { ... }
    pub fn kv_delete(&self, key: &[u8]) -> Result<(), ServiceError> { ... }
    pub fn kv_contains(&self, key: &[u8]) -> Result<bool, ServiceError> { ... }
}
```

Each method builds the appropriate `SectorRequest::Kv*` variant and dispatches
via `self.sector_dispatch.dispatch(...)`, following the same pattern as the
existing `get`/`put`/`len` methods.

### Tests

- Round-trip: `kv_put` then `kv_get` returns the value.
- `kv_contains` returns `false` before insert, `true` after.
- `kv_delete` removes the key; subsequent `kv_get` returns `None`.
- KV operations are isolated per `ProgramId`.
- Existing append-only log tests still pass.

---

## 4. Point-to-Point Messaging

**Priority:** P1
**Crates touched:** `grid-core`, `grid-net`, `grid-service`, `zode`

### Problem

The only point-to-point mechanism is the sector request-response protocol.
Services that need directed communication (state sync, targeted requests)
must either misuse `SectorRequest` or broadcast everything via GossipSub.

### Design

Add a generic direct-message protocol to `grid-net` using libp2p
request-response, tagged with a topic string so the Zode can route to the
correct `ServiceGossipHandler`.

### Implementation

#### 4a. Define wire types — `crates/grid-core/src/direct_message.rs` (new file)

```rust
use serde::{Deserialize, Serialize};

/// A direct message between two Zodes, tagged with a topic for routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectMessage {
    pub topic: String,
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
}

/// Acknowledgement for a direct message (fire-and-forget semantics).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectMessageAck {
    pub ok: bool,
}
```

Re-export from `crates/grid-core/src/lib.rs`.

#### 4b. Add request-response protocol — `crates/grid-net/src/behaviour.rs`

Add a second request-response behaviour alongside the existing `sector_rr`:

```rust
#[derive(NetworkBehaviour)]
pub(crate) struct GridBehaviour {
    pub(crate) gossipsub: gossipsub::Behaviour,
    pub(crate) sector_rr: request_response::cbor::Behaviour<SectorRequest, SectorResponse>,
    pub(crate) direct_rr: request_response::cbor::Behaviour<DirectMessage, DirectMessageAck>,
    // ...
}
```

Protocol ID: `/grid/direct/1.0.0`.

#### 4c. Add NetworkEvent variants — `crates/grid-net/src/event.rs`

```rust
pub enum NetworkEvent {
    // ... existing variants ...

    /// An incoming direct message from a peer.
    IncomingDirectMessage {
        peer: ZodeId,
        message: DirectMessage,
        channel: ResponseChannel<DirectMessageAck>,
    },

    /// Response for an outbound direct message.
    DirectMessageResult {
        peer: ZodeId,
        request_id: OutboundRequestId,
        response: DirectMessageAck,
    },

    /// An outbound direct message failed.
    DirectMessageFailure {
        peer: ZodeId,
        request_id: OutboundRequestId,
        error: String,
    },
}
```

#### 4d. Add send method — `crates/grid-net/src/service.rs`

```rust
impl NetworkService {
    pub fn send_direct(
        &mut self,
        peer: &ZodeId,
        message: DirectMessage,
    ) -> OutboundRequestId {
        self.swarm
            .behaviour_mut()
            .direct_rr
            .send_request(peer, message)
    }

    pub fn send_direct_ack(
        &mut self,
        channel: ResponseChannel<DirectMessageAck>,
        ack: DirectMessageAck,
    ) -> Result<(), NetworkError> {
        self.swarm
            .behaviour_mut()
            .direct_rr
            .send_response(channel, ack)
            .map_err(|_| NetworkError::ResponseFailed)
    }
}
```

#### 4e. Dispatch in Zode — `crates/zode/src/zode.rs`

Handle `IncomingDirectMessage` in `dispatch_event`.  Route to the
`ServiceGossipHandler` that claims the message's topic (reuse `on_gossip`
or add an `on_direct` method to the trait).  Send back `DirectMessageAck { ok: true }`.

#### 4f. Expose in ServiceContext — `crates/grid-service/src/context.rs`

Add a channel for outbound direct messages, similar to `publish_tx`:

```rust
impl ServiceContext {
    pub fn send_direct(
        &self,
        peer_id: &str,
        topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), ServiceError> { ... }
}
```

Internally sends a `(ZodeId, DirectMessage)` tuple through an mpsc channel
that the Zode event loop processes.

### Tests

- Send a direct message between two Zodes; verify the handler receives it.
- Direct messages on unhandled topics are ack'd but dropped.
- Timeout / failure propagation.

---

## 5. Node Identity in ServiceContext

**Priority:** P2
**Crates touched:** `grid-service`, `zode`

### Problem

Services have no access to the node's identity.  Any service that needs to
sign data or identify itself to peers must manage its own key material.

### Design

Expose a read-only `NodeIdentity` through `ServiceContext` that wraps the
Zode's existing Ed25519 keypair.

### Implementation

#### 5a. Define NodeIdentity — `crates/grid-service/src/identity.rs` (new file)

```rust
/// Read-only view of the Zode's node identity.
///
/// Provides the node's peer ID and Ed25519 signing capability.
pub struct NodeIdentity {
    zode_id: String,
    public_key: Vec<u8>,
    signing_fn: Arc<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>,
}

impl NodeIdentity {
    pub fn new(
        zode_id: String,
        public_key: Vec<u8>,
        signing_fn: Arc<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>,
    ) -> Self {
        Self { zode_id, public_key, signing_fn }
    }

    pub fn zode_id(&self) -> &str { &self.zode_id }
    pub fn public_key(&self) -> &[u8] { &self.public_key }
    pub fn sign(&self, data: &[u8]) -> Vec<u8> { (self.signing_fn)(data) }
}
```

#### 5b. Add to ServiceContext

```rust
pub struct ServiceContext {
    // ... existing fields ...
    identity: Arc<NodeIdentity>,
}

impl ServiceContext {
    pub fn identity(&self) -> &NodeIdentity { &self.identity }
}
```

#### 5c. Build in Zode startup

In `Zode::start`, after creating the `NetworkService`, extract the keypair's
Ed25519 signing key and build a `NodeIdentity`.  Pass it through
`ServiceRegistry` into each `ServiceContext`.

The libp2p `Keypair` is already available.  Use `keypair.sign(data)` for the
signing closure.

### Tests

- `identity().zode_id()` matches the Zode's actual peer ID.
- `identity().sign(data)` produces a valid Ed25519 signature verifiable with
  `identity().public_key()`.

---

## 6. Proof Registry in ServiceContext

**Priority:** P2
**Crates touched:** `grid-service`, `zode`

### Problem

Proof verification is only accessible inside `SectorRequestHandler`
(`crates/zode/src/sector_handler.rs`).  Services that need to verify proofs
as a standalone operation (e.g. verifying a spend proof before admitting a
transaction to a mempool) cannot access the proof infrastructure.

### Design

Pass a shared reference to the existing `ProofVerifierRegistry` through
`ServiceContext`.

### Implementation

#### 6a. Add to ServiceContext — `crates/grid-service/src/context.rs`

```rust
use grid_proof::ProofVerifierRegistry;

pub struct ServiceContext {
    // ... existing fields ...
    proof_registry: Option<Arc<ProofVerifierRegistry>>,
}

impl ServiceContext {
    /// Access the proof verifier registry for standalone proof verification.
    ///
    /// Returns `None` if the Zode was started without proof verification
    /// support (e.g. in test configurations).
    pub fn proof_registry(&self) -> Option<&Arc<ProofVerifierRegistry>> {
        self.proof_registry.as_ref()
    }
}
```

#### 6b. Wire in Zode

The `ProofVerifierRegistry` is already built in `Zode::start` (around line
112-121 of `crates/zode/src/zode.rs`).  Wrap it in `Arc` and pass it through
`ServiceRegistry` to `ServiceContext`.

Add a new field on `ServiceRegistry`:

```rust
pub struct ServiceRegistry {
    // ... existing fields ...
    proof_registry: Option<Arc<ProofVerifierRegistry>>,
}
```

Set it alongside `sector_dispatch` in `start_all`.

#### 6c. Add grid-proof dependency

`grid-service` needs a new dependency on `grid-proof` in its `Cargo.toml`:

```toml
[dependencies]
grid-proof = { path = "../grid-proof" }
```

This introduces a new edge in the dependency graph.  It is safe because
`grid-proof` is a leaf crate that depends only on `grid-core`.

### Tests

- `proof_registry()` returns `Some` when the Zode is started with proof keys.
- A service can call `registry.verify(...)` and receive the expected
  `VerifiedSector` or `ProofError`.

---

## Implementation Order

```
Phase 1 (P0 — required for any custom-protocol service):
  1. ServiceGossipHandler trait + Zode dispatch
  2. Publish / Subscribe in ServiceContext

Phase 2 (P1 — required before production):
  3. KV Storage in ProgramStore
  4. Point-to-Point Messaging

Phase 3 (P2 — quality of life):
  5. Node Identity in ServiceContext
  6. Proof Registry in ServiceContext
```

Each phase is independently shippable.  Phase 1 items have no dependencies
on each other but should land in the same release since services typically
need both gossip handling and publishing together.

---

## Backwards Compatibility

All changes are additive.  Existing services (Identity, Interlink) do not
implement `ServiceGossipHandler` and do not call publish/subscribe, so they
continue to work without modification.  The new `SectorStore` KV methods
have default implementations that return `StorageError::Unsupported` so
existing `SectorStore` implementations compile without changes until they
opt in.

The new RocksDB column family (`service_kv`) is created automatically by
`create_missing_column_families(true)` which is already set in
`RocksStorage::open`.  Existing databases gain the empty column family on
first open after upgrade.
