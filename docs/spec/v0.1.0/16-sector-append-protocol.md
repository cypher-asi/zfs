# ZFS v0.1.0 — Sector Append Protocol

Replaces the key-value sector protocol (13-sector-requirements) with an
append-only log model. No backward compatibility with v1 blob put/get.

## 1. Design

Sectors are append-only logs — ordered sequences of individually encrypted
entries indexed 0, 1, 2, ... All operations are append/read. "Single-value"
use cases read the latest entry.

**Core identity:** sector = channel = shared-secret bucket = unit of Zode
subscription.

## 2. Storage Model

### 2.1 Key Layout

RocksDB column family `sectors`, composite key:

```
pid (32 B) || sid (32 B) || index (8 B big-endian)
```

Total key size: 72 bytes. Sector IDs must be exactly 32 bytes; Zodes reject
non-32-byte IDs.

### 2.2 Operations

| Operation   | Description |
|-------------|-------------|
| `append`    | Find max stored index via reverse-seek, write at next index, return it. |
| `insert_at` | Store at a specific index if absent (idempotent, for gossip replication). |
| `read_log`  | Forward-iterate from `from_index`, return up to `max_entries` values. |
| `log_length` | Reverse-seek to find max index + 1 (0 if empty). |
| `sector_stats` | Full scan: count sectors (unique pid+sid), entries, and total bytes. |
| `list_programs` | Deduplicated forward scan over pid prefix. |
| `list_sectors` | Prefix-iterate over pid, deduplicate sid. |

### 2.3 Trait

```rust
pub trait SectorStore {
    fn append(&self, pid: &ProgramId, sid: &SectorId, entry: &[u8]) -> Result<u64, StorageError>;
    fn insert_at(&self, pid: &ProgramId, sid: &SectorId, index: u64, entry: &[u8]) -> Result<bool, StorageError>;
    fn read_log(&self, pid: &ProgramId, sid: &SectorId, from: u64, max: usize) -> Result<Vec<Vec<u8>>, StorageError>;
    fn log_length(&self, pid: &ProgramId, sid: &SectorId) -> Result<u64, StorageError>;
    fn sector_stats(&self) -> Result<SectorStorageStats, StorageError>;
    fn list_programs(&self) -> Result<Vec<ProgramId>, StorageError>;
    fn list_sectors(&self, pid: &ProgramId) -> Result<Vec<SectorId>, StorageError>;
}
```

## 3. Wire Protocol

Protocol string: `/zfs/sector/2.0.0`.

### 3.1 Request/Response

```rust
enum SectorRequest {
    Append(SectorAppendRequest),
    ReadLog(SectorReadLogRequest),
    LogLength(SectorLogLengthRequest),
    BatchAppend(SectorBatchAppendRequest),
    BatchLogLength(SectorBatchLogLengthRequest),
}
```

Responses mirror request variants 1:1.

### 3.2 Batch Limits

- Maximum **64 entries** per batch.
- Maximum **4 MB total payload** per batch.

### 3.3 Single Append

Fields: `program_id`, `sector_id`, `entry` (encrypted bytes).
Response: `ok`, `index` (assigned), optional `error_code`.

### 3.4 ReadLog

Fields: `program_id`, `sector_id`, `from_index`, `max_entries`.
Response: `entries` (byte arrays), optional `error_code`.

### 3.5 LogLength

Fields: `program_id`, `sector_id`.
Response: `length`, optional `error_code`.

## 4. Gossip

### 4.1 GossipSectorAppend

```rust
struct GossipSectorAppend {
    program_id: ProgramId,
    sector_id: SectorId,
    index: u64,
    payload: Vec<u8>,
}
```

Published to `prog/{pid_hex}` on successful local append.

### 4.2 Receiving Zode Behavior

1. Check program subscription.
2. Check sector filter (if `AllowList`, sector must be in the set).
3. Check entry size limit.
4. `insert_at(pid, sid, index, payload)` — store if index absent (idempotent).

## 5. Sector Filter

```rust
enum SectorFilter {
    All,
    AllowList(HashSet<SectorId>),
}
```

Per-Zode filter applied after program-level topic check. Default: `All`.
Handler checks filter before append, read_log, and gossip acceptance.
Unsubscribed sectors are silently dropped.

## 6. Chat (Interlink) Integration

- One sector per channel; sector ID derived from `ChannelId::sector_id()`.
- `send_message`: encrypt → `storage.append(pid, sid, ciphertext)` → publish
  `GossipSectorAppend` with assigned index.
- Background updater: poll `log_length()`, fetch new entries via
  `read_log(pid, sid, known_len, 64)`, decrypt each, send to UI.
- No per-message sector IDs; no read-modify-write.

## 7. Storage UI

- Sector displayed as collapsible log with entry count: `"abc123... (42 entries)"`.
- Each entry shown as sub-item with index, size, and hex/text preview.
- Stats show both sector count and total entry count.

## 8. Crate Responsibilities

| Crate | Changes |
|-------|---------|
| `zfs-storage` | New `SectorStore` trait (append/insert_at/read_log/log_length). New key layout. |
| `zfs-core` | `GossipSectorAppend`, `SectorAppendRequest/Response`, `SectorReadLogRequest/Response`, `SectorLogLengthRequest/Response`, batch variants. |
| `zfs-zode` | Handler dispatches append/read_log/log_length. Gossip decodes `GossipSectorAppend`, stores via `insert_at`. `SectorFilter` in config. |
| `zfs-zode-app` | Chat uses single sector per channel. Updater polls `log_length`. Storage UI shows log entries. |
| `zfs-sdk` | `sector_append`, `sector_read_log`, `sector_log_length` replace `sector_store`/`sector_fetch`. |

## 9. Removed

- `put` / `get` / `batch_put` / `batch_get` storage operations.
- `SectorStoreRequest` / `SectorFetchRequest` / `SectorBatchStoreRequest` / `SectorBatchFetchRequest`.
- `GossipSector` (replaced by `GossipSectorAppend`).
- `SectorBatchEntry`, `SectorPutResult` types.
- `StorageError::SlotOccupied`, `StorageError::ConditionFailed`.
- `overwrite` / `expected_hash` fields (no CAS semantics).
