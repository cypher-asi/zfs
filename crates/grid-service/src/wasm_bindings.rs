// Host import implementations for the WASM service runtime.
//
// When the WASM module calls `store.get(program_id, key)`, the host:
// 1. Reconstructs a `ProgramId` from the bytes
// 2. Derives `SectorId = SHA-256(key)` via `ProgramStore`
// 3. Calls `SectorDispatch::dispatch(SectorRequest::ReadLog { ... })`
// 4. Returns the last entry (or None)
//
// This is the same ProgramStore logic used by native services, exposed
// across the WASM boundary. Full implementation requires the `wasm`
// feature flag and `wasmtime`.
