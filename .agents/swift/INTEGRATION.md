# Swift / Rust coordination

## Verified baseline — 2026-09-11

- `apps/macos/Package.swift` uses Swift tools 6.0 and targets macOS 14+.
- SwiftPM's `CMilkyFFI` system-library target wraps a handwritten C header.
- Rust is linked as a static library. `milky-probe` calls only
  `uint32_t milky_abi_version(void)`; the current ABI version is 1.
- Rust has an app search engine and in-memory usage store. No search, engine
  handle, selection event, or result-memory API currently crosses the boundary.
- Native `milky-launcher` now runs with explicit `--fixtures`, independently of
  `CMilkyFFI`. `MilkyNative.SearchProvider` is the future adapter seam.
- Presentation result values currently carry a stable string ID, display name,
  and application URL. This Swift-local shape does not prescribe ABI layout.

Source of truth: the live Rust exports, C header, and package configuration.
Recheck them before integration rather than treating this snapshot as permanent.

## Ownership

The core/pairing owner implements Rust exports and coordinates matching C header
changes. The Swift owner implements the Swift adapter, panel, and native actions.
Agree on a boundary change before either side depends on it. Record agreed
cross-language ownership and ABI decisions in the shared DECISIONS.md, then link
them here. Do not invent C signatures or modify the header alone.

## Next contract to agree — proposal, not implemented

The first integration needs:

1. Engine construction and destruction, including initialization failure.
2. Search with query and result limit, returning ordered results with stable
   identity/path and display name. Match kind is optional presentation/debug data.
3. Explicit string encoding, allocation ownership, result lifetime, and freeing.
4. Error behavior and supported threading: who may call the engine, on which
   executor, and whether calls may overlap. Define shutdown with work in flight.
5. A subsequent local impression/selection event contract: distinguish selection,
   action attempt, and action success; coordinate persistence with the core owner.

Do not mark events as persisted until the real path exists. The current usage
store is not an event log and does not establish a cross-language event schema.

## Independent Swift progress

Use a small Swift search-provider abstraction with fixture and real adapters.
Fixture results may contain stable IDs, display names, and app URLs; they are
presentation models and do not prescribe the C memory layout. Exercise delayed,
out-of-order, empty, and failed responses as well as successful search.

Keep mock mode explicit. Once the contract exists, convert results to Swift-owned
values before releasing foreign memory, following the agreed ownership rules.
Never declare a handle Sendable merely to silence concurrency errors.

## Pending coordination

No search ABI has been agreed yet. Add concrete proposals here only when needed,
with owner, status, and the unresolved question. Keep completed agreements as
links to the shared decision rather than maintaining two competing specifications.
