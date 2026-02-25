# ZFS v0.1.0 — Standard programs (ZID and Interlink)

## Purpose

This document defines the two standard programs for v0.1.0: **ZID (Zero Identity)** and **Interlink**. Both use `ProgramDescriptor`, canonical message formats, and optional proof/signature requirements. Where these live (e.g. `zfs-programs` or dedicated modules) and encoding (e.g. canonical CBOR) are specified.

## Default programs

ZID and Interlink are **default programs**: every Zode subscribes to them out of the box. Operators can toggle individual default programs on or off in the Zode settings (see [06-zode § Default programs](06-zode.md#default-programs)). This allows lean deployments that serve only a subset of the standard programs or only custom programs.

## ZID (Zero Identity)

- **Description:** Minimal identity program; versioned; canonical encoding; optional ZK conformance.
- **ProgramDescriptor:** Includes at least: program name/type (e.g. "zid"), version (e.g. 1), optional `proof_required: bool`.
- **Message types:** e.g. `ZidMessage` — identity claim or update; canonical CBOR.
- **Proof requirements:** Optional ZK conformance; if required, Valid-Sector proof per [04-proof](04-proof.md).
- **Size limits:** Implementation-defined (e.g. max message size per program).

## Interlink

- **Description:** Structured messages; canonical encoding; size bounds; optional signature policy (inside or outside ZK).
- **ProgramDescriptor:** Includes at least: program name/type (e.g. "zchat"), version, optional proof/signature flags.
- **Message types:** e.g. `ZChatMessage` — structured chat message; canonical CBOR; fields (sender, content, timestamp, etc.) implementation-defined.
- **Signature policy:** Optional signature validation (in-ZK or out-of-ZK); spec defines message format and size bounds; signature policy is optional/TBD.
- **Size limits:** Max message size (e.g. 64 KiB) per message; document in implementation.

### Channels and storage layout

- **Channel:** A Interlink **channel** is a logical stream of messages. It is identified by a **ChannelId** (canonical bytes, e.g. CBOR-encoded string or fixed-size id). One **sector** per channel: `SectorId = canonical(ChannelId)` (or a dedicated encoding such as `"zchat/channel/" || channel_id_bytes`).
- **Head per channel:** The sector head store (see [02-storage](02-storage.md)) holds one **Head** per sector (hence per channel). `Head.sector_id` = channel’s SectorId, `Head.cid` = Cid of the **latest** message block for that channel, `Head.prev_head_cid` = previous head Cid for lineage (optional; enables walking history backwards).
- **Block store:** Each stored block is one encrypted **ZChatMessage** (canonical CBOR then encrypted by client). Key = Cid (hash of ciphertext); value = ciphertext. Program index keys by ProgramId (Interlink) and lists Cids for that program.
- **Who can decrypt:** Sector payloads are encrypted client-side (see [10-crypto](10-crypto.md)). **Only clients that possess the SectorKey** (and nonce, if not carried with the ciphertext) can decrypt. Zodes never see plaintext and do not perform decryption. For Interlink, therefore **only participants who have the channel key** can decrypt messages in that channel. Key distribution (e.g. how channel keys are agreed or shared) is implementation-defined and out of scope for this spec.

## Shared requirements

- **ProgramDescriptor:** Both use the base `ProgramDescriptor` from [11-core-types](11-core-types.md); canonical encoding and `program_id()` / `topic()` per [03-programs-and-topics](03-programs-and-topics.md).
- **Canonical message format:** CBOR (deterministic) for all message types; same choice as core types and protocol.
- **Proof requirement flags:** Per-program; Zode and SDK use these to decide whether to require proof (see [04-proof](04-proof.md), [06-zode](06-zode.md), [09-sdk](09-sdk.md)).
- **What proofs prove:** When a program requires a Valid-Sector proof, the proof attests to the **program’s actual message fields** (e.g. ZidMessage vs ZChatMessage have different schemas). The Zode verifies without seeing plaintext; verifier keys and proof logic are per program because the fields and validity rules differ. See [04-proof](04-proof.md).

## Interfaces (summary)

```rust
// ZID
pub struct ZidDescriptor { /* extends ProgramDescriptor */ }
pub struct ZidMessage { /* identity claim/update; canonical CBOR */ }

// Interlink
pub struct ZChatDescriptor { /* extends ProgramDescriptor */ }
pub struct ZChatMessage {
    // e.g. sender, content, timestamp; canonical CBOR
    // size limit: e.g. 64 KiB
}

// Channel: one sector per channel; SectorId derived from ChannelId
pub type ChannelId = Vec<u8>;  // or fixed size; canonical encoding
fn sector_id_for_channel(channel_id: &ChannelId) -> SectorId {
    // e.g. canonical CBOR of ("zchat", channel_id) or PREFIX || channel_id
}
```

- **Size limits:** Document in crate (e.g. `ZChatMessage::MAX_SIZE`).
- **Proof requirement:** `proof_required: bool` or enum in descriptor; Zode enforces when storing.
- **Test channel:** For zode-app test messages (see [08-zode-app](08-zode-app.md)), a reserved channel id (e.g. `"INTERLINK-MAIN"` or a fixed byte string) may be used so test traffic does not collide with real channels.

## Diagrams (optional)

- **Message flow:** Client builds ZidMessage/ZChatMessage → canonical encode → encrypt (if sector payload) → store with optional proof.
- **Validation:** Zode receives store → (optional) verify proof → validate message format and size → persist.

## Implementation

- **Location:** `zfs-programs` or dedicated modules (e.g. `zfs-programs::zid`, `zfs-programs::zchat`). Same crate as program identity and topic.
- **Encoding:** Canonical CBOR for descriptors and messages; consistent with [11-core-types](11-core-types.md) and [12-protocol](12-protocol.md).
- **SDK helpers:** [09-sdk](09-sdk.md) provides ZID and Interlink helper APIs (e.g. create descriptor, build message, upload).
