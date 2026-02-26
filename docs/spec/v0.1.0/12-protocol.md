# The Grid v0.1.0 — Protocol (v1)

## Purpose

This document defines the v1 protocol: message types, wire format, transport (how store/fetch are sent), discovery, replication semantics, and cryptographic authentication. Implemented in `grid-net` (wire) and used by `zode` and `grid-sdk`.

## Message types

| Message | Direction | Description |
|---------|-----------|-------------|
| **StoreRequest** | Client → Zode | Store a block (ciphertext) and optional head; optional proof; signature + machine DID. |
| **StoreResponse** | Zode → Client | Success or rejection (with error code). |
| **FetchRequest** | Client → Zode | Request block or head by Cid or sector; optionally signed. |
| **FetchResponse** | Zode → Client | Block ciphertext, head, or error. |

All messages may be wrapped in an **envelope** with `program_id`, `version`, and/or topic for routing and validation.

## Wire format

- **Serialization:** Canonical CBOR (same as [11-core-types](11-core-types.md)) for all protocol messages.
- **Consistency:** Same CBOR library and deterministic encoding as core types and storage.

### StoreRequest

```rust
pub struct StoreRequest {
    pub program_id: ProgramId,
    pub cid: Cid,
    pub ciphertext: Vec<u8>,
    pub head: Option<Head>,
    pub proof: Option<Vec<u8>>,
    pub key_envelope: Option<KeyEnvelope>,  // wrapped SectorKey(s) for recipients
    // --- signing fields ---
    pub machine_did: String,                // did:key of the signing machine
    pub signature: HybridSignature,         // Ed25519 (64 B) + ML-DSA-65 (3,309 B)
}
```

### Signed payload

- **What is signed:** canonical CBOR encoding of `(program_id, cid, optional head hash, timestamp)` — prevents replay and binds the signature to the content.
- **Signature format:** `HybridSignature` from `zid` — always contains both Ed25519 (64 bytes) and ML-DSA-65 (3,309 bytes). Raw binary serialization: `ed25519 (64 B) || ml_dsa (3,309 B)` via `HybridSignature::to_bytes()`/`from_bytes()`. Within CBOR protocol messages, the signature fields (`ed25519: [u8; 64]`, `ml_dsa: Vec<u8>`) are encoded as CBOR byte strings.
- **Verification:** Zodes verify via `MachinePublicKey::verify(msg, sig)`, which checks **both** Ed25519 and ML-DSA-65 components. The `MachinePublicKey` is resolved from `machine_did` (see [10-crypto](10-crypto.md) for DID encoding).
- **Wire cost:** hybrid signatures add ~3.3 KB per signed message; envelope entries add ~1.2 KB per recipient. For store-heavy workloads this is acceptable; the sector ciphertext itself is typically much larger.

### StoreResponse

```rust
pub struct StoreResponse {
    pub ok: bool,
    pub error_code: Option<GridError>,  // if !ok
}
```

### FetchRequest

```rust
pub struct FetchRequest {
    pub program_id: ProgramId,
    pub by_cid: Option<Cid>,
    pub by_sector_id: Option<SectorId>,
    // Optional signing (lower priority; Zode may serve anyone)
    pub machine_did: Option<String>,
    pub signature: Option<HybridSignature>,
}
```

### FetchResponse

```rust
pub struct FetchResponse {
    pub ciphertext: Option<Vec<u8>>,
    pub head: Option<Head>,
    pub error_code: Option<GridError>,
}
```

### Head updates

`Head` gains an optional `signature: HybridSignature` field so head lineage has cryptographic attribution. Field names match the canonical definition in [11-core-types](11-core-types.md):

```rust
pub struct Head {
    pub sector_id: SectorId,
    pub cid: Cid,
    pub version: u64,
    pub program_id: ProgramId,
    pub prev_head_cid: Option<Cid>,
    pub timestamp_ms: u64,
    // --- added for protocol ---
    pub signature: Option<HybridSignature>,  // signed by the updating machine
}
```

### Verification policy

Zodes can optionally verify `signature` against `machine_did` if they track known identities via ZID:

- **Strict mode:** reject unsigned StoreRequests.
- **Permissive mode:** accept unsigned StoreRequests (default for v0.1.0).

Policy is configurable per Zode.

## Transport

- **Discovery:** See [Discovery](#discovery). Clients discover Zode peers via bootstrap list, config, DHT, or mDNS.
- **Connection:** libp2p + QUIC. Handshake/identify as per libp2p.
- **Store / Fetch:** Sent as **request-response** over libp2p request-response protocol. Topic (GossipSub) is used for **subscription** and possibly for broadcast/announce; actual store/fetch payloads use request-response so the client gets a direct StoreResponse/FetchResponse.
- **Topic usage:** Zodes subscribe to `prog/{program_id}` (see [03-programs-and-topics](03-programs-and-topics.md)). Requests may be routed or validated using program_id; topic subscription does not replace request-response for store/fetch.

## Discovery

- **Bootstrap peers:** Configurable list of multiaddrs (e.g. in config file or env). Client and Zode can use bootstrap to find the network.
- **Config file:** Zode and SDK config may include `bootstrap_peers: Vec<Multiaddr>`.
- **DHT / mDNS:** Optional; implementation may use libp2p Kademlia DHT or mDNS for peer discovery. Not mandated for v0.1.0; bootstrap list is sufficient.
- **Connect API:** `connect(bootstrap_peers, config) -> Result<Connection, Error>` (conceptual). SDK uses this to connect to Zodes before sending store/fetch.

## Replication semantics

- **Replication factor R:** Client chooses R (e.g. number of Zodes to store to). Passed as parameter in SDK (see [09-sdk](09-sdk.md)).
- **Partial success:** Semantics are **at least one success** for store: if at least one of R Zodes accepts, the client may consider the store successful. Optional stricter mode "all R" can be implementation-defined.
- **Fetch:** Client may fetch from any Zode that has the Cid; no consensus. First successful FetchResponse wins (or implementation-defined strategy).

## Interfaces (summary)

- **Message structs:** `StoreRequest`, `StoreResponse`, `FetchRequest`, `FetchResponse` (and optional envelope).
- **Serialization:** `encode_canonical` / `decode_canonical` (CBOR).
- **Discovery:** `bootstrap_peers` in config; `connect(peers, config)` in `grid-net` API.
- **Send/receive:** Implemented in `grid-net`; Zode and SDK use the same API (send store request, receive store response, etc.).

## Sequence diagrams

### Client store (signed, to R Zodes)

```mermaid
sequenceDiagram
    participant C as Client
    participant Z1 as Zode 1
    participant Z2 as Zode 2
    C->>C: Sign(program_id, cid, head_hash, timestamp)
    C->>Z1: StoreRequest + HybridSignature + machine_did
    C->>Z2: StoreRequest + HybridSignature + machine_did
    Z1->>Z1: Verify signature (if strict)
    Z2->>Z2: Verify signature (if strict)
    Z1-->>C: StoreResponse
    Z2-->>C: StoreResponse
```

### Client fetch (by Cid)

```mermaid
sequenceDiagram
    participant C as Client
    participant Z as Zode
    C->>Z: FetchRequest(by_cid)
    Z-->>C: FetchResponse(ciphertext/head or error)
```

### Discovery flow

```mermaid
sequenceDiagram
    participant C as Client
    participant Config as Config
    participant Net as grid-net
    Config->>Net: bootstrap_peers
    Net->>Net: connect to peers
    Net-->>C: Connection ready
```

## Implementation

- **Crate:** `grid-net`. Implements wire format, request-response, and discovery; used by `zode` and `grid-sdk`.
- **06-zode and 09-sdk** reference this spec for message shapes and replication semantics.
- **Signing:** `zid` types (`HybridSignature`, `MachineKeyPair`, `MachinePublicKey`) are used for request signing and verification. SDK signs via `MachineKeyPair::sign()`; Zodes verify via `MachinePublicKey::verify()`. See [10-crypto](10-crypto.md) for full API.
