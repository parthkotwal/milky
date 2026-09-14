# Swift / Rust coordination

## Historical fixture baseline — 2026-09-11

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

## Adapter and fixture rules

Use a small Swift search-provider abstraction with fixture and real adapters.
Fixture results may contain stable IDs, display names, and app URLs; they are
presentation models and do not prescribe the C memory layout. Exercise delayed,
out-of-order, empty, and failed responses as well as successful search.

Keep mock mode explicit. Once the contract exists, convert results to Swift-owned
values before releasing foreign memory, following the agreed ownership rules.
Never declare a handle Sendable merely to silence concurrency errors.

## Agreed contract — 2026-09-13

Rust side implemented 2026-09-13 at ABI version 2; Swift adapter connected
2026-09-13 in `Sources/MilkyRust/RustSearchProvider.swift`. Verified from outside Swift: a C program linked against
`libmilky_ffi.a` made 1000 requests with correct results, and `leaks` reported
0 leaks. The header in `Sources/CMilkyFFI/include/milky.h` matches the exports.

**Build warning:** Bare SwiftPM does not relink when only the Rust archive changes.
`milky-probe` kept printing ABI 1 after Rust moved to 2 until its binary was
deleted and rebuilt. `scripts/build.sh` now builds Rust, fingerprints the archive,
and cleans Swift products when its contents change. `scripts/run.sh` uses this
path. Bare `swift build/run/test` bypasses the protection; use the scripts after
Rust edits. The runtime version check remains in the adapter.

Authoritative text: shared
[`../DECISIONS.md`](../DECISIONS.md), entry "Swift ↔ Rust boundary:
message-based JSON over a five-function C ABI". Do not restate or fork it here.

Swift-side summary for building the adapter:

- Link `libmilky_ffi.a`; check `milky_abi_version() == 2` before creating an engine.
- `milky_engine_new()` once; NULL means initialization failure. Free only at
  termination, never while a request is in flight.
- Per search: encode `{"op":"search","query":...,"limit":...}`, call
  `milky_engine_request`, copy the returned bytes into Swift-owned `Data`, call
  `milky_string_free` exactly once, then decode. Never keep the foreign pointer.
- `kind: "error"` means failure (show error UI); `kind: "search"` with empty
  `results` means no matches. Map `path` to `AppResult.id` and `url`, `name` to
  `name`; `match_kind` is a string, debug/presentation only.
- Calls may overlap on one engine from any thread. Keep generation checks; the
  engine does not cancel.

Owners: Rust exports and `milky.h` — core owner (the user writes the Rust side,
learning-first). Swift adapter and linking — Swift owner.

## Pending coordination

- Impression/selection event messages: not designed. They will be new `op`
  values in the same envelope, not new C functions.


## Verified Swift adapter — 2026-09-13

- Real Rust search is the default. Only `--fixtures` selects development data;
  engine failures show errors with Retry and never trigger fixture fallback.
- `RustSearchProvider` holds its engine in an actor, prepares off the UI thread,
  serializes its own short synchronous calls, and skips cancelled queued work.
  The contract permits concurrency; this adapter does not require it. Existing
  presentation generations still discard obsolete responses.
- Every foreign response is copied into Data, freed with `defer`, then decoded.
  Order and display names come directly from Rust; path is stable identity.
  Relative/NUL-containing paths, duplicate IDs, mismatched query echoes, unknown
  reply kinds, and invalid JSON fail visibly rather than becoming zero results.
- App termination cancels presentation work and awaits actor shutdown before
  replying to AppKit's termination request. Engine destruction cannot overlap
  its synchronous request. A shut-down provider cannot recreate an engine.
- Native launcher links the same Rust archive even in fixture mode, so the
  fixture launch wrapper now also builds Rust (without calling it for searches).
- Verified real Visual Studio Code and three IDLE results in the running app,
  real no-match response, and Enter launching Calculator. 12 Swift tests pass,
  including actual ABI/engine requests, post-free result lifetime, concurrent
  callers, escaped input, wire errors, unsupported versions, and shutdown.
- No usage event messages or persistence were added. App discovery is still the
  Rust engine's startup snapshot; restart Milky to pick up installed/removed apps.
