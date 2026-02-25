# ZFS v0.1.0 — Zode (node)

## Purpose

The Zode is the storage node: it runs libp2p + QUIC, uses GossipSub for topic subscription, persists blocks and heads in RocksDB via `zfs-storage`, verifies proofs when required, enforces local storage policy, and exposes metrics/state for the UI. This document defines requirements, config, storage policy, and integration with proof and verifier key storage.

## Requirements (must)

- **Run libp2p + QUIC:** Transport and discovery via `zfs-net`; no direct libp2p outside `zfs-net`.
- **Use GossipSub:** Subscribe to configured program topics (`prog/{program_id}`).
- **Subscribe to configured program topics:** Only accept store/fetch for subscribed programs (policy).
- **Persist in RocksDB:** All persistence via `zfs-storage` only (BlockStore, HeadStore, ProgramIndex). No direct RocksDB.
- **Verify proofs when required:** Before persisting a block, if the program requires proof, call `ProofVerifier::verify`; reject with `ProofInvalid` on failure (see [04-proof](04-proof.md), [11-core-types](11-core-types.md)).
- **Enforce local storage policy:** See [Storage policy](#storage-policy).
- **Expose metrics/state to UI:** Counters and gauges (e.g. blocks stored, peer count, storage usage) for [07-zode-cli](07-zode-cli.md) and [08-zode-app](08-zode-app.md).

## Config schema

Full Zode config (single place for node behavior):

| Field | Type | Description |
|-------|------|-------------|
| **storage** | StorageConfig | Path, max_open_files, compression, max_db_size_bytes (see [02-storage](02-storage.md)). |
| **default_programs** | DefaultProgramsConfig | Toggle default programs on or off (see [Default programs](#default-programs)). |
| **topics** | Vec\<ProgramId\> or Vec\<String\> | Additional (non-default) program topics to subscribe to. |
| **limits** | LimitsConfig | Max size per program, max total DB size (for policy). |
| **proof_policy** | ProofPolicyConfig | When to require proof; path or config for verifier key store (see [04-proof](04-proof.md)). |
| **network** | NetworkConfig | Listen address, bootstrap_peers, etc. |

**Storage config:** Same as [02-storage](02-storage.md): path, max_open_files, compression, max_db_size_bytes.

**Proof config:** Base path or config for verifier key storage (e.g. `program_store_path` or `verifier_key_path`). Passed to `zfs-proof` for loading verifier keys (see [04-proof](04-proof.md)).

**Format:** Config file (YAML/TOML) and/or env vars; implementation-defined. Document in crate.

## Default programs

ZFS ships with **default programs** — the standard programs defined in [05-standard-programs](05-standard-programs.md) that a Zode subscribes to out of the box. In v0.1.0 the default programs are **ZID** and **Interlink**.

Default programs are **enabled by default** but can be individually toggled off in the Zode settings. This lets operators run lean nodes that serve only specific workloads (e.g. ZID-only, or only custom programs via `topics`).

### DefaultProgramsConfig

```rust
pub struct DefaultProgramsConfig {
    pub zid: bool,    // default: true
    pub zchat: bool,  // default: true
}
```

When a default program is **enabled**, the Zode automatically subscribes to its topic and accepts store/fetch requests for it — the operator does not need to add its `program_id` to the `topics` list manually. When **disabled**, the Zode does not subscribe and rejects requests for that program.

### Effective topic list

At startup the Zode computes the effective set of subscribed programs:

```
effective_topics = { p.program_id() | p ∈ default_programs, p.enabled }
                 ∪ { t | t ∈ config.topics }
```

The `topics` field is reserved for **additional** (non-default) programs. An operator who only wants the defaults can leave `topics` empty.

### Settings persistence

Changes to `default_programs` are persisted to the config file (or equivalent store). Both the CLI and the standalone app expose a **Settings** screen where these toggles are presented (see [07-zode-cli](07-zode-cli.md) and [08-zode-app](08-zode-app.md)).

## Storage policy

Concrete rules the Zode enforces (reject with `PolicyReject` or `StorageFull` when violated):

| Rule | Description |
|------|-------------|
| **Program allowlist** | Only accept store/fetch for programs in the **effective topic list** (enabled default programs + config `topics`). Reject with `PolicyReject` for other programs. |
| **Max size per program** | Optional cap (bytes or block count) per program_id; reject with `StorageFull` or `PolicyReject` when exceeded. |
| **Max total DB size** | Optional cap (max_db_size_bytes in storage config); reject with `StorageFull` when at capacity. |
| **Eviction** | v0.1.0 does not mandate eviction; if implemented, document (e.g. LRU per program or global). |

Policy is enforced in the Zode request handler **before** or **after** proof verification (e.g. check program allowlist first, then verify proof, then check size limits before persisting).

## Interfaces

- **Zode config:** Struct as above; load from file/env.
- **Hooks:** Storage (via `zfs-storage`), proof (via `ProofVerifier`), policy (check program + limits before/after persist).
- **Metrics surface:** Counters (e.g. `blocks_stored_total`, `store_rejections_total` by reason), gauges (e.g. `peer_count`, `db_size_bytes`). Exposed for UI (see [07-zode-cli](07-zode-cli.md)).

## State machine (lifecycle)

```mermaid
stateDiagram-v2
    direction LR
    [*] --> Init
    Init --> LoadConfig: load config
    LoadConfig --> OpenStorage: open RocksDB via zfs-storage
    OpenStorage --> StartNet: start libp2p (zfs-net)
    StartNet --> SubscribeTopics: subscribe to program topics
    SubscribeTopics --> Serving: accept store/fetch
    Serving --> [*]
```

## Request flow (StoreRequest)

```mermaid
stateDiagram-v2
    direction LR
    [*] --> ReceiveRequest
    ReceiveRequest --> CheckPolicy: program in topics? size limits?
    CheckPolicy --> Reject: PolicyReject / StorageFull
    CheckPolicy --> VerifyProof: if proof required
    VerifyProof --> Reject: ProofInvalid
    VerifyProof --> Persist: put block, head, program index
    Persist --> [*]
```

## Implementation

- **Crate:** `zfs-zode`. Deps: zfs-core, zfs-crypto, zfs-programs, zfs-proof, zfs-net, zfs-storage.
- **No direct RocksDB:** Call `zfs-storage` only.
- **Config:** Config file and env; document schema in crate and reference [02-storage](02-storage.md) for storage, [04-proof](04-proof.md) for verifier key path.
- **Verifier key storage:** Zode passes config (e.g. `program_store_path`) to proof layer; proof crate loads verifier keys from that location (see [04-proof](04-proof.md)).
