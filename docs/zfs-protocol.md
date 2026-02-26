# The ZFS Protocol

**Version 0.2.0** — Derived from the reference implementation

---

## 1. Introduction

ZFS (Zero File System) is a peer-to-peer encrypted storage protocol. Participating nodes — called **Zodes** — form a network of program-scoped, append-only log stores. Clients encrypt all data locally before upload; Zodes store and serve only opaque ciphertext. The protocol has no consensus layer, no tokens, and no global state.

This document defines the ZFS protocol in implementation-neutral terms. Any conforming implementation — regardless of programming language — that follows the wire formats, cryptographic constructions, and behavioral rules described here can interoperate with any other conforming implementation.

---

## 2. Terminology

| Term | Definition |
|------|-----------|
| **Zode** | A storage node that participates in the ZFS network. Identified by a libp2p PeerId, displayed with a `Zx` prefix (e.g. `Zx12D3KooW…`). The prefix is display-only; the wire and storage formats use the raw PeerId. |
| **Client** | Any participant that connects to Zodes to store or retrieve data. A client does not serve requests. |
| **Program** | A named, versioned application scope. All storage, subscriptions, and topic routing are scoped by program. |
| **ProgramId** | A 32-byte identifier derived as `SHA-256(CBOR(ProgramDescriptor))`. |
| **Sector** | An append-only log of encrypted entries identified by a `(ProgramId, SectorId)` pair. |
| **SectorId** | An opaque 32-byte identifier. In the sector protocol, sector IDs MUST be exactly 32 bytes. |
| **Entry** | A single encrypted blob appended to a sector log. Entries are indexed 0, 1, 2, ... |
| **NeuralKey** | A 256-bit root secret from which all identity and machine keys are deterministically derived. |
| **SectorKey** | A random 256-bit symmetric key used to encrypt sector entries. Algorithm is XChaCha20-Poly1305 or Poseidon sponge per program (§11). |
| **Topic** | A GossipSub topic string of the form `prog/<program_id_hex>`, used for subscription and data propagation. |

---

## 3. Serialization

All protocol messages, program descriptors, and gossip payloads use **CBOR** (RFC 8949) with deterministic encoding (RFC 8949 §4.2.1).

Implementations MUST produce identical byte sequences for identical logical values. Field ordering within CBOR maps MUST be consistent. The canonical CBOR bytes are the input to all hash operations (ProgramId derivation, CID computation, etc.).

Binary fields (payloads, keys, ciphertexts) are encoded as CBOR byte strings.

---

## 4. Identifiers

### 4.1 ProgramId

A 32-byte value:

```
ProgramId = SHA-256(CBOR(ProgramDescriptor))
```

Displayed as 64 lowercase hex characters.

### 4.2 ProgramDescriptor

A CBOR map with at minimum:

| Field | Type | Description |
|-------|------|-------------|
| `name` | text string | Short program name (e.g. `"zid"`, `"zchat"`) |
| `version` | unsigned integer | Program version number |
| `proof_required` | boolean | Whether Valid-Sector proofs are required |
| `proof_system` | ProofSystem enum | Encryption and proof backend: `None` (XChaCha20-Poly1305) or `Groth16` (Poseidon sponge + shape proofs). See §11 and §17. |

Program-specific descriptors MAY add additional fields. Two implementations that serialize the same descriptor fields to CBOR MUST produce the same ProgramId.

### 4.3 SectorId

A variable-length byte string. In the sector protocol, sector IDs MUST be exactly 32 bytes. Zodes MUST reject requests with non-32-byte sector IDs.

Displayed as lowercase hex.

### 4.4 Cid (Content Identifier)

```
Cid = SHA-256(ciphertext)
```

A 32-byte content-addressed identifier derived from the stored ciphertext. Used for content addressing outside the sector protocol (e.g. in future block-based protocols). Displayed as 64 lowercase hex characters.

### 4.5 ZodeId

A libp2p PeerId. On the wire and in storage, the raw PeerId bytes are used. For human display, the canonical format is `Zx<PeerId_string>`. The `Zx` prefix MUST NOT appear in wire formats.

---

## 5. Topic Naming

Programs are addressed on the network via GossipSub topics:

```
topic = "prog/" + hex(ProgramId)
```

Where `hex(ProgramId)` is the 64-character lowercase hex encoding of the 32-byte ProgramId.

Zodes subscribe to one or more program topics and only accept requests for subscribed programs.

---

## 6. Transport Layer

### 6.1 Connection

ZFS uses **libp2p** as its network substrate with the following transports:

- **QUIC** (`/quic-v1`): Primary transport.
- **TCP + Noise + Yamux**: Fallback transport.

The default listen address is `/ip4/0.0.0.0/udp/0/quic-v1`.

### 6.2 Protocols

Three libp2p sub-protocols run on the same swarm:

| Protocol String | Type | Purpose |
|----------------|------|---------|
| `/zfs/sector/1.0.0` | Request-response (CBOR) | Client ↔ Zode sector operations |
| `/zfs/kad/1.0.0` | Kademlia DHT | Peer discovery |
| GossipSub | Pub/sub | Data propagation across program topics |

Sector requests and responses are serialized as CBOR and exchanged via libp2p's request-response protocol.

### 6.3 GossipSub Configuration

- **Message authentication**: Signed (libp2p keypair).
- **Heartbeat interval**: 10 seconds.
- **Validation mode**: Permissive.
- **Message ID**: Derived from the hash of the message data (content-based deduplication).

### 6.4 Peer Discovery

#### 6.4.1 Bootstrap Peers

Zodes accept a list of bootstrap peer multiaddrs in their configuration. On startup, the Zode dials each bootstrap peer.

#### 6.4.2 Kademlia DHT

When enabled, the Zode seeds the Kademlia routing table with bootstrap peers, triggers an initial `bootstrap()`, and periodically performs random walk queries (`get_closest_peers(random_peer_id)`) at a configurable interval (default: 30 seconds).

Kademlia configuration:

| Parameter | Default |
|-----------|---------|
| Protocol name | `/zfs/kad/1.0.0` |
| Query timeout | 60 seconds |
| Mode | `Server` for Zodes, `Client` for SDK clients |
| Random walk interval | 30 seconds |
| Max concurrent discovery dials | 8 |

Newly discovered peers are automatically dialed (up to the concurrency limit). Once connected, GossipSub and request-response operate automatically over the new peer.

---

## 7. Sector Protocol — Wire Format

The sector protocol is the primary client-to-Zode interface. It operates over `/zfs/sector/1.0.0` as a request-response protocol.

### 7.1 Request Envelope

All requests are wrapped in a top-level enum:

```
SectorRequest = Append(SectorAppendRequest)
              | ReadLog(SectorReadLogRequest)
              | LogLength(SectorLogLengthRequest)
              | BatchAppend(SectorBatchAppendRequest)
              | BatchLogLength(SectorBatchLogLengthRequest)
```

### 7.2 Response Envelope

```
SectorResponse = Append(SectorAppendResponse)
               | ReadLog(SectorReadLogResponse)
               | LogLength(SectorLogLengthResponse)
               | BatchAppend(SectorBatchAppendResponse)
               | BatchLogLength(SectorBatchLogLengthResponse)
```

Response variants correspond 1:1 to request variants.

### 7.3 Append

Append a single encrypted entry to a sector log.

**Request:**

| Field | Type | Description |
|-------|------|-------------|
| `program_id` | bytes(32) | Target program |
| `sector_id` | bytes(32) | Target sector |
| `entry` | byte string | Encrypted payload |

**Response:**

| Field | Type | Description |
|-------|------|-------------|
| `ok` | boolean | Whether the append succeeded |
| `index` | optional uint64 | The assigned log index (present on success) |
| `error_code` | optional ErrorCode | Present on failure |

### 7.4 ReadLog

Read entries from a sector log starting at a given index.

**Request:**

| Field | Type | Description |
|-------|------|-------------|
| `program_id` | bytes(32) | Target program |
| `sector_id` | bytes(32) | Target sector |
| `from_index` | uint64 | First entry index to read |
| `max_entries` | uint32 | Maximum entries to return (capped at 64) |

**Response:**

| Field | Type | Description |
|-------|------|-------------|
| `entries` | array of byte strings | The requested entries, in index order |
| `error_code` | optional ErrorCode | Present on failure |

If the sector does not exist or `from_index` is past the end, an empty `entries` array is returned (not an error).

### 7.5 LogLength

Query the number of entries in a sector log.

**Request:**

| Field | Type | Description |
|-------|------|-------------|
| `program_id` | bytes(32) | Target program |
| `sector_id` | bytes(32) | Target sector |

**Response:**

| Field | Type | Description |
|-------|------|-------------|
| `length` | uint64 | Number of entries (0 if sector does not exist) |
| `error_code` | optional ErrorCode | Present on failure |

### 7.6 BatchAppend

Append entries to multiple sectors under one program in a single round trip.

**Request:**

| Field | Type | Description |
|-------|------|-------------|
| `program_id` | bytes(32) | Target program (shared across all entries) |
| `entries` | array of BatchAppendEntry | Up to 64 entries |

Each `BatchAppendEntry`:

| Field | Type | Description |
|-------|------|-------------|
| `sector_id` | bytes(32) | Target sector |
| `entry` | byte string | Encrypted payload |

**Response:**

| Field | Type | Description |
|-------|------|-------------|
| `results` | array of AppendResult | Parallel to `entries` |

Each `AppendResult`:

| Field | Type | Description |
|-------|------|-------------|
| `ok` | boolean | Success flag |
| `index` | optional uint64 | Assigned index |
| `error_code` | optional ErrorCode | Error detail |

### 7.7 BatchLogLength

Query log lengths for multiple sectors under one program.

**Request:**

| Field | Type | Description |
|-------|------|-------------|
| `program_id` | bytes(32) | Target program |
| `sector_ids` | array of bytes(32) | Up to 64 sector IDs |

**Response:**

| Field | Type | Description |
|-------|------|-------------|
| `results` | array of LogLengthResult | Parallel to `sector_ids` |
| `error_code` | optional ErrorCode | Batch-level error only |

Each `LogLengthResult`:

| Field | Type | Description |
|-------|------|-------------|
| `length` | uint64 | Entry count |
| `error_code` | optional ErrorCode | Per-sector error |

### 7.8 Batch Limits

- Maximum **64 entries** per batch request.
- Maximum **4 MB total payload** per batch request.

Zodes MUST reject batches exceeding these limits with `BatchTooLarge`.

### 7.9 Error Codes

Responses use a shared set of error codes:

| Code | Meaning |
|------|---------|
| `StorageFull` | Zode's storage capacity exceeded |
| `ProofInvalid` | Valid-Sector proof verification failed |
| `PolicyReject` | Request rejected by Zode policy (program not subscribed, sector not in allowlist) |
| `NotFound` | Requested data not present |
| `InvalidPayload` | Malformed request or entry exceeds size limit |
| `ProgramMismatch` | Program ID does not match subscription |
| `SlotOccupied` | Reserved (write-once semantics) |
| `BatchTooLarge` | Batch exceeds entry count or payload size limit |
| `ConditionFailed` | Reserved (conditional write semantics) |

Error codes are serialized as CBOR text strings.

---

## 8. Gossip Replication

When a Zode accepts an `Append` request from a client, it publishes a `GossipSectorAppend` message to the program's GossipSub topic (`prog/<program_id_hex>`). Other Zodes subscribed to that topic store the entry automatically.

### 8.1 GossipSectorAppend

| Field | Type | Description |
|-------|------|-------------|
| `program_id` | bytes(32) | Program this entry belongs to |
| `sector_id` | bytes(32) | Sector this entry belongs to |
| `index` | uint64 | The assigned log index |
| `payload` | byte string | The encrypted entry bytes |

Serialized as canonical CBOR and published to GossipSub.

### 8.2 Receiving Zode Behavior

When a Zode receives a `GossipSectorAppend`:

1. **Program check**: If the Zode does not serve `program_id`, discard.
2. **Sector filter**: If the Zode uses an allowlist, check `sector_id`. Discard if not in the set.
3. **Entry size check**: If the payload exceeds the Zode's max entry size limit, discard.
4. **Idempotent insert**: Store the entry at the specified `index` if that index is not already occupied. If the index already exists, silently ignore (idempotent).
5. **No re-gossip**: The Zode does NOT re-publish the message. GossipSub's mesh propagation handles fan-out.

---

## 9. Zode Storage Model

### 9.1 Append-Only Logs

Each sector is an ordered sequence of entries indexed starting at 0. Sectors are identified by a composite key `(ProgramId, SectorId)`.

### 9.2 Key Layout

The storage key for a single entry is:

```
key = program_id (32 bytes) || sector_id (32 bytes) || index (8 bytes, big-endian)
```

Total key size: 72 bytes. All entries for a sector share the same 64-byte prefix.

### 9.3 Operations

| Operation | Behavior |
|-----------|----------|
| **append** | Find the maximum stored index via reverse-seek on the sector prefix, write at `max + 1`, return the new index. |
| **insert_at** | Write at a specific index only if that index does not already exist (idempotent). Used for gossip replication. |
| **read_log** | Forward-iterate from `from_index` within the sector prefix, return up to `max_entries` values. |
| **log_length** | Reverse-seek to find the maximum stored index + 1. Returns 0 if the sector is empty. |

### 9.4 Policy Enforcement

Zodes enforce the following policies:

| Policy | Description |
|--------|-------------|
| **Program allowlist** | Only serve programs in the effective topic set (enabled default programs + explicit configuration). Reject with `PolicyReject`. |
| **Sector filter** | Optionally restrict to an explicit set of allowed sector IDs. Default: all sectors accepted. |
| **Entry size limit** | Maximum payload size per entry. Default: 256 KB. Reject with `InvalidPayload`. |
| **Batch limits** | Max 64 entries, max 4 MB total payload per batch. Reject with `BatchTooLarge`. |
| **Per-program quota** | Optional maximum bytes per program. |

---

## 10. Cryptographic Primitives

### 10.1 Key Hierarchy

All keys derive from a **NeuralKey** — a 256-bit secret generated by CSPRNG.

```
NeuralKey (256-bit CSPRNG)
│
├── IdentitySigningKey (Ed25519 + ML-DSA-65)
│   Derived with: identity_id (16 bytes)
│
└── MachineKeyPair (per device, per epoch)
    Derived with: identity_id (16 bytes), machine_id (16 bytes), epoch (uint64)
    Contains:
    ├── Ed25519      — classical signing
    ├── X25519       — classical key agreement
    ├── ML-DSA-65    — post-quantum signing
    └── ML-KEM-768   — post-quantum key encapsulation
```

All four key types are always present in every MachineKeyPair. There is no classical-only mode.

### 10.2 Key Derivation

All derivation uses **HKDF-SHA256** with `salt = None` unless otherwise specified.

#### Identity Signing Key

Two separate derivations from the NeuralKey:

| Component | HKDF info |
|-----------|-----------|
| Ed25519 seed | `"cypher:id:identity:v1" \|\| identity_id` |
| ML-DSA-65 seed | `"cypher:id:identity:pq-sign:v1" \|\| identity_id` |

#### Machine Key Pair

Two-level derivation:

**Step 1 — Machine seed:**

```
machine_seed = HKDF-SHA256(
    ikm  = NeuralKey,
    info = "cypher:shared:machine:v1" || identity_id || machine_id || epoch_be_bytes
)
```

**Step 2 — Individual key seeds** (from machine seed):

| Component | HKDF info |
|-----------|-----------|
| Ed25519 signing | `"cypher:shared:machine:sign:v1" \|\| machine_id` |
| X25519 encryption | `"cypher:shared:machine:encrypt:v1" \|\| machine_id` |
| ML-DSA-65 signing | `"cypher:shared:machine:pq-sign:v1" \|\| machine_id` |
| ML-KEM-768 encapsulation | `"cypher:shared:machine:pq-encrypt:v1" \|\| machine_id` |

ML-KEM-768 keys are generated deterministically: the PQ encrypt seed is expanded via HKDF into `d` and `z` parameters:

| Parameter | HKDF info (from PQ encrypt seed) |
|-----------|----------------------------------|
| d | `"mlkem768:d"` |
| z | `"mlkem768:z"` |

These are passed to ML-KEM-768 deterministic key generation.

### 10.3 Machine Key Capabilities

Machine keys carry a bitflag:

| Bit | Name | Value |
|-----|------|-------|
| 0 | SIGN | 0x01 |
| 1 | ENCRYPT | 0x02 |
| 2 | STORE | 0x04 |
| 3 | FETCH | 0x08 |

Capabilities are metadata and not enforced cryptographically — enforcement is at the application/policy layer.

### 10.4 Hybrid Signatures

A **HybridSignature** always contains both components:

| Component | Size | Description |
|-----------|------|-------------|
| Ed25519 | 64 bytes | Classical signature |
| ML-DSA-65 | 3,309 bytes | Post-quantum signature |

**Binary format:** `ed25519_bytes (64) || ml_dsa_bytes (3309)`

Both components are produced by signing the same message. **Both MUST verify** for the signature to be considered valid. If either fails, the signature is rejected.

### 10.5 Hybrid Key Encapsulation

Key agreement combines X25519 and ML-KEM-768:

```
x25519_ss  = X25519(sender_x25519_secret, recipient_x25519_public)
(mlkem_ct, mlkem_ss) = ML-KEM-768.Encapsulate(recipient_mlkem_public)

shared_secret = HKDF-SHA256(
    ikm  = x25519_ss || mlkem_ss,
    salt = None,
    info = "zero-neural:encap:v1"
) → 32 bytes
```

The output is a `SharedSecret` (32 bytes) and an `EncapBundle`:

| Field | Size | Description |
|-------|------|-------------|
| `x25519_public` | 32 bytes | Sender's static X25519 public key |
| `mlkem_ciphertext` | 1,088 bytes | ML-KEM-768 ciphertext |

**Binary format:** `x25519_public (32) || mlkem_ciphertext (1088)`

An attacker must break **both** X25519 and ML-KEM-768 to recover the shared secret. If the X25519 DH produces an all-zero shared secret, the operation MUST fail.

### 10.6 DID Encoding

Ed25519 public keys are encoded as `did:key` identifiers:

```
did:key:z + base58btc(0xed01 || ed25519_public_key_bytes)
```

The multicodec prefix `0xed01` identifies Ed25519 public keys.

### 10.7 Shamir Secret Sharing

The NeuralKey MAY be split into Shamir shares for backup and recovery:

- **Split**: `split(secret[32], total, threshold, rng) → shares[total]`
- **Combine**: `combine(shares[≥ threshold]) → secret[32]`

Each share is serialized as hex: `hex(index_byte || share_data)`.

Identity generation, signing, and machine key derivation can all operate by **ephemerally reconstructing** the NeuralKey from shares, performing the operation, and immediately zeroizing the secret.

---

## 11. Sector Encryption

Sector encryption supports two algorithms. The choice is per-program via `proof_system` in the ProgramDescriptor (§4.2): `None` uses XChaCha20-Poly1305; `Groth16` uses Poseidon sponge (required for shape proofs, §17).

### 11.1 SectorKey

A SectorKey is a random 256-bit symmetric key generated via CSPRNG. It is the key material for encrypting and decrypting sector entries. Key wrapping (§11.5) is unchanged regardless of encryption algorithm.

### 11.2 XChaCha20-Poly1305 (proof_system = None)

**Algorithm:** XChaCha20-Poly1305 (AEAD)

```
sealed = nonce (24 bytes) || ciphertext || tag (16 bytes)
```

- **Nonce**: 192-bit, randomly generated per encryption.
- **AAD** (Associated Authenticated Data): `program_id (32 bytes) || sector_id (32 bytes)`. This binds the ciphertext to its program and sector, preventing cross-program and cross-sector ciphertext relocation.
- The nonce is prepended to the ciphertext in the sealed output.

### 11.3 Poseidon Sponge (proof_system = Groth16)

**Parameters:** BN254 scalar field, rate=2, capacity=1.

**Sealed format:**

```
sealed = nonce (32 bytes) || ciphertext_elements (32 bytes each) || tag (32 bytes)
```

- **Nonce**: 256-bit, randomly generated per encryption.
- **AAD**: `program_id (32 bytes) || sector_id (32 bytes)` absorbed into the sponge before the plaintext.
- **Ciphertext**: Plaintext is split into 32-byte (256-bit) field elements; sponge absorbs key and nonce, then XORs/absorbs plaintext elements. Output elements form `ciphertext_elements`.
- **Tag**: Final 32-byte output authenticates the entire encryption (AAD + ciphertext).

The Poseidon sponge construction enables shape-proof circuits (§17) that prove ciphertext integrity without revealing plaintext.

### 11.4 Padding

Before encryption, content MUST be padded to fixed-size buckets to resist payload-size analysis. Buckets grow in powers of two (2× progression):

| Content size (including 4-byte length prefix) | Padded to |
|-----------------------------------------------|-----------|
| 0 – 256 bytes | 256 B |
| 257 – 512 bytes | 512 B |
| 513 – 1,024 bytes | 1 KB |
| 1,025 – 2,048 bytes | 2 KB |
| 2,049 – 4,096 bytes | 4 KB |
| 4,097 – 8,192 bytes | 8 KB |
| 8,193 – 16,384 bytes | 16 KB |
| 16,385 – 32,768 bytes | 32 KB |
| 32,769 – 65,536 bytes | 64 KB |
| 65,537 – 131,072 bytes | 128 KB |
| 131,073 – 262,144 bytes | 256 KB |
| > 262,144 bytes | Next 256 KB multiple |

**Padding format:** length-prefix + zero-fill:

```
padded = content_length (4 bytes, little-endian) || content || 0x00 * (bucket_size - 4 - content_length)
```

On decryption, the receiver reads the 4-byte length prefix, extracts `content[0..length]`, and discards trailing zeros.

### 11.5 Key Wrapping

To share a SectorKey with a recipient, the sender performs a two-step wrap:

**Step 1 — Hybrid key agreement:**

Performed between the sender's MachineKeyPair and the recipient's MachinePublicKey using the hybrid encapsulation described in §10.5. This produces a `SharedSecret` and an `EncapBundle`.

**Step 2 — Context-bound wrapping:**

```
wrap_key = HKDF-SHA256(
    ikm  = SharedSecret,
    salt = None,
    info = "zfs:sector-key-wrap:v1" || program_id || sector_id
)

wrapped_sector_key = XChaCha20-Poly1305(
    key       = wrap_key,
    nonce     = random 192-bit,
    plaintext = sector_key_bytes
)
```

The result is a **KeyEnvelopeEntry**:

| Field | Type | Description |
|-------|------|-------------|
| `recipient_did` | text string | `did:key` of the recipient machine |
| `sender_x25519_public` | byte string | Sender's static X25519 public key (32 bytes) |
| `mlkem_ciphertext` | byte string | ML-KEM-768 ciphertext (1,088 bytes) |
| `wrapped_key` | byte string | `nonce (24) \|\| encrypted_sector_key (32) \|\| tag (16)` = 72 bytes |

Each entry is approximately 1,192 bytes (dominated by the ML-KEM ciphertext).

### 11.6 Sector ID Derivation

For metadata-private storage, sector IDs are derived client-side via two-step HKDF:

```
derivation_key = HKDF-SHA256(
    ikm  = shared_secret,
    salt = "zfs:sector:v1",
    info = "zfs:sector:derive-key:v1"
)

sector_id = HKDF-SHA256(
    ikm  = derivation_key,
    salt = "zfs:sector:v1",
    info = <application-defined info string>
)
```

The `shared_secret` is typically the raw bytes of a SectorKey. A derivation key is first extracted to ensure the SectorKey is never used directly as HKDF input for sector IDs (it is reserved for AEAD encryption). The intermediate derivation key MUST be zeroized after use.

**Info string convention:**

```
"zfs:{program_short_name}:{purpose}:{...application_fields}"
```

Examples:

| Application | Info string |
|-------------|-------------|
| Chat channel sector | `"zfs:zchat:inbox:{group_id_hex}:{seq}"` |
| ZID profile | `"zfs:zid:profile:{identity_id_hex}"` |

**Properties:**

- **Deterministic**: All parties sharing the same secret and info string compute the same sector ID.
- **Unlinkable**: Different info strings produce unrelated sector IDs.
- **Collision-resistant**: 32-byte HKDF output; negligible collision probability.

---

## 12. Standard Programs

### 12.1 ZID (Zero Identity)

**Descriptor:**

| Field | Value |
|-------|-------|
| `name` | `"zid"` |
| `version` | `1` |
| `proof_required` | `false` |

**ZidMessage:**

| Field | Type | Description |
|-------|------|-------------|
| `owner_did` | text string | DID of the identity owner |
| `display_name` | optional text string | Human-readable name |
| `timestamp_ms` | uint64 | Milliseconds since Unix epoch |

### 12.2 Interlink (ZChat)

**Descriptor:**

| Field | Value |
|-------|-------|
| `name` | `"zchat"` |
| `version` | `1` |
| `proof_required` | `false` |

**ZChatMessage:**

| Field | Type | Description |
|-------|------|-------------|
| `sender_did` | text string | DID of the sender |
| `channel_id` | byte string | Channel identifier |
| `content` | text string | Message text (UTF-8) |
| `timestamp_ms` | uint64 | Milliseconds since Unix epoch |

Maximum encoded message size: 64 KB.

**Channel-to-sector mapping:**

```
SectorId = SHA-256("zchat/channel/" || channel_id_bytes)
```

**Message-to-sector mapping** (per-message sector for write-once entries):

```
SectorId = SHA-256("zchat/msg/" || channel_id_bytes || "/" || timestamp_ms_le || "/" || sender_did)
```

**Reserved test channel:** `"INTERLINK-MAIN"` — used for test traffic in development.

Both ZID and Interlink are **default programs**: Zodes subscribe to them out of the box. Operators can individually disable either program.

---

## 13. Zode Behavior

### 13.1 Startup Sequence

1. Open persistent storage.
2. Start the network (libp2p swarm with GossipSub, request-response, and optionally Kademlia).
3. Compute the effective topic set: enabled default programs ∪ explicitly configured programs.
4. Subscribe to each topic on GossipSub.
5. Dial bootstrap peers.
6. If Kademlia is enabled, seed the routing table and trigger initial bootstrap.
7. Enter the event loop.

### 13.2 Event Loop

The Zode continuously processes:

- **Incoming sector requests** (`/zfs/sector/1.0.0`): Dispatch to the appropriate handler (Append, ReadLog, LogLength, BatchAppend, BatchLogLength), enforce policy, store if accepted, and send the response.
- **Gossip messages** (GossipSub): Deserialize `GossipSectorAppend` from CBOR, store via idempotent `insert_at`.
- **Peer events**: Track connected/disconnected peers.
- **Discovery events** (Kademlia): Auto-dial newly discovered peers.
- **Publish queue**: Drain outbound gossip publishes.
- **Shutdown signal**: Graceful exit.

### 13.3 Append + Gossip Flow

When a Zode accepts an Append request:

1. Validate program, sector filter, and entry size.
2. Append the entry to local storage, receiving the assigned index.
3. Send `SectorAppendResponse(ok=true, index)` to the client.
4. Serialize a `GossipSectorAppend` with the program_id, sector_id, assigned index, and payload.
5. Publish to `prog/<program_id_hex>` via GossipSub.

The client is responsible for triggering the gossip publish by calling the Zode's publish interface after a successful append.

---

## 14. Visibility and Privacy Properties

### What a Zode can see

| Information | Visible? |
|-------------|----------|
| `program_id` | Yes — used for routing and policy |
| `sector_id` | Yes — opaque 32 bytes; cannot be reversed without the shared secret |
| Payload size (post-encryption, post-padding) | Yes — mitigated by fixed-size padding buckets |
| Entry timing | Yes — when appends and reads arrive |
| Which sector IDs are batched together | Yes — within a single batch request |
| Client IP address | Yes — transport-level |

### What a Zode cannot see

| Information | Why |
|-------------|-----|
| Entry content or structure | Encrypted with XChaCha20-Poly1305 or Poseidon sponge |
| Who wrote an entry | Identity is inside the encrypted payload or absent entirely |
| Who can read an entry | Key material never appears on the wire |
| Which sectors are related | HKDF-derived sector IDs are unlinkable without the shared secret |
| Ordering or timestamps | Inside the encrypted payload |

---

## 15. Interoperability Requirements

A conforming ZFS implementation MUST:

1. Serialize all protocol messages as deterministic CBOR (RFC 8949 §4.2.1).
2. Derive ProgramIds as `SHA-256(CBOR(ProgramDescriptor))` using identical CBOR encoding.
3. Use the protocol string `/zfs/sector/1.0.0` for sector request-response.
4. Use the protocol string `/zfs/kad/1.0.0` for Kademlia DHT.
5. Format GossipSub topics as `prog/<64_hex_chars>`.
6. Serialize `GossipSectorAppend` as CBOR for gossip propagation.
7. Use XChaCha20-Poly1305 (proof_system=None) or Poseidon sponge (proof_system=Groth16) for sector encryption, per ProgramDescriptor. XChaCha20-Poly1305 uses 192-bit random nonces; Poseidon uses the format in §11.3.
8. Implement the padding bucket scheme (§11.4) for all encrypted payloads.
9. Use HKDF-SHA256 with the exact domain separation strings specified in §10.2 for key derivation.
10. Produce hybrid signatures containing both Ed25519 (64 bytes) and ML-DSA-65 (3,309 bytes) components, and require both to verify.
11. Perform hybrid key encapsulation using X25519 + ML-KEM-768 combined via HKDF (§10.5).
12. Reject sector IDs that are not exactly 32 bytes.
13. Enforce batch limits: 64 entries maximum, 4 MB total payload maximum.
14. Accept `GossipSectorAppend` idempotently: store at the given index only if not already occupied.

---

## 16. Security Considerations

- **No transport anonymity.** IP addresses are visible to Zodes and network observers. Onion routing is out of scope.
- **Timing correlation.** A Zode can correlate writes and reads from the same connection. Batch requests reveal co-accessed sector IDs.
- **No wire-level write authorization.** Any client that can connect and knows a ProgramId can append to any sector within that program. Access control is an application-layer concern.
- **Gossip propagation delay.** Between client append and gossip delivery, the entry exists on only one Zode. Clients MAY multi-send to multiple Zodes for faster availability.
- **Key compromise.** If a NeuralKey is compromised, all derived identity and machine keys are compromised. Shamir splitting mitigates single-point-of-failure for the root secret.
- **Post-quantum readiness.** All signatures and key agreements use hybrid constructions (classical + post-quantum). An attacker must break both the classical and post-quantum components.

---

## 17. Shape Proofs

Shape proofs allow clients to prove that an encrypted blob conforms to a declared schema without revealing plaintext. Programs with `proof_system = Groth16` use Poseidon sponge encryption (§11.3) and MAY attach a shape proof to each entry.

### 17.1 ProofSystem Enum

| Variant | Encryption | Shape proofs |
|---------|------------|--------------|
| `None` | XChaCha20-Poly1305 | Not supported |
| `Groth16` | Poseidon sponge | Supported |

### 17.2 FieldSchema

Message structure is described by a **FieldSchema** for use in circuits and verification:

| Type | Description |
|------|-------------|
| `CborType` | CBOR type identifier (integer, text, bytes, array, map, etc.) |
| `FieldDef` | Named field: `(name, CborType)` |
| `FieldSchema` | Sequence of `FieldDef` defining the message shape |

**Schema hash:**

```
schema_hash = SHA-256(canonical_cbor(schema))
```

### 17.3 ShapeProof Wire Format

| Field | Type | Description |
|-------|------|-------------|
| `proof_system` | enum | `Groth16` |
| `ciphertext_hash` | bytes(32) | Poseidon(C) — hash of the ciphertext C |
| `proof_bytes` | byte string | Groth16 proof |
| `schema_hash` | bytes(32) | SHA-256 of canonical CBOR schema |
| `size_bucket` | enum | Message-size bucket (see §17.6) |

### 17.4 Universal Circuit

The shape-proof circuit proves three predicates:

1. **shape(B)** — Decrypted plaintext B conforms to the declared FieldSchema.
2. **Poseidon_encrypt(B, K, N, AAD) = C** — Ciphertext C is the Poseidon encryption of B under key K, nonce N, and AAD.
3. **Poseidon(C) = ciphertext_hash** — The public `ciphertext_hash` matches Poseidon(C).

AAD = `program_id || sector_id` as in §11.3.

### 17.5 Strong Binding

The Zode computes `Poseidon(received_ciphertext)` and MUST verify that it equals the attested `ciphertext_hash`. This binds the proof to the exact ciphertext stored; tampering invalidates the binding.

### 17.6 Message-Size Buckets

Proofs are generated per size bucket. One trusted setup (proving key, verification key) exists per bucket.

| Bucket | Max message size |
|--------|------------------|
| 1 KB | 1,024 bytes |
| 4 KB | 4,096 bytes |
| 16 KB | 16,384 bytes |
| 64 KB | 65,536 bytes |

**Version 1:** Only 1 KB and 4 KB buckets are supported.

### 17.7 Trusted Setup

One (pk, vk) pair per bucket. Setup is program-agnostic; the same keys apply across programs using the same bucket.

### 17.8 Verification Rules

1. **Proof validity**: Groth16 proof verifies against the vk for the declared `size_bucket`.
2. **Ciphertext binding**: `Poseidon(received_ciphertext) == ciphertext_hash` from the proof.
3. **Schema consistency**: `schema_hash` matches the program's declared schema for the message type.
4. **Bucket match**: Entry size (post-padding) fits within `size_bucket`.

---

## 18. Message Signing

Message signing provides authenticity and integrity for entries within the encrypted blob. Signatures are PQ-hybrid (Ed25519 + ML-DSA-65) and are produced before encryption, verified after decryption.

### 18.1 Placement

Signatures live **inside** the encrypted payload. The Zode never sees plaintext; signature verification is end-to-end between sender and recipient. Only parties with the SectorKey can decrypt and verify.

### 18.2 Signable Bytes

For a message with a signature field:

```
signable_bytes = canonical_cbor(all_fields_except_signature)
```

Fields MUST be serialized in deterministic CBOR order. The signature field is excluded from the input to the signing function.

### 18.3 HybridSignature Format

| Component | Size | Description |
|-----------|------|-------------|
| Ed25519 | 64 bytes | Classical signature |
| ML-DSA-65 | 3,309 bytes | Post-quantum signature |

**Binary format:** `Ed25519 (64) || ML-DSA-65 (3309)` = 3,373 bytes total

Both components sign the same `signable_bytes`. **Both MUST verify** for the signature to be valid.

### 18.4 Signing and Verification Flow

1. **Signing**: Client computes `signable_bytes`, signs with Ed25519 and ML-DSA-65, appends `HybridSignature` to the message, then encrypts the full blob.
2. **Verification**: Recipient decrypts, extracts the signature, recomputes `signable_bytes` from the remaining fields, and verifies both Ed25519 and ML-DSA-65.

### 18.5 Ed25519-Only Fallback (v1)

For version 1 compatibility, programs MAY support **Ed25519-only** signatures (64 bytes). Recipients that support hybrid verification SHOULD accept Ed25519-only if ML-DSA-65 is absent and the program descriptor indicates v1 fallback mode.
