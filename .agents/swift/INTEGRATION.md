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

## Core changes — 2026-10-08

- Result `name` is now the display name Finder shows, from LaunchServices
  ("Find My", "Zoom", "Voice Memos"), not the bundle folder name. Paths, the
  identity, are unchanged.
- `milky_engine_new` takes about 23 ms longer: display-name lookups for ~127
  apps. Keep creating the engine off the main thread.
- `libmilky_ffi.a` now needs CoreFoundation and IOKit. The core owner added
  `link framework` lines for both to `Sources/CMilkyFFI/module.modulemap`, so
  every target importing `CMilkyFFI` links them automatically. Verified with
  `scripts/build.sh test`: 21 Swift tests pass; `milky-probe` reports ABI 2.
- Matching is now accent-insensitive and camelCase-aware. Result order still
  comes entirely from Rust; no Swift change is required.
- 2026-10-08: settings also match Apple's keywords, and `match_kind` can be
  `"keyword"` (only keywords matched; the title is the destination's default
  title). Additive, ABI stays 3; Swift ignores `match_kind`. Verified with
  `scripts/build.sh test`: 22 Swift tests pass.

## Agreed integration

### Contract v3 — agreed and wired 2026-10-08

Authoritative text: shared [`../DECISIONS.md`](../DECISIONS.md), "Results carry
a kind, an action, and a destination ID (contract v3)". Swift-side summary:

- `milky_abi_version()` becomes 3. Results replace `path`/`name` with `id`,
  `kind` (`app` | `setting`), `title`, `subtitle`, and `action`
  (`{"type":"launch","path":...}` or `{"type":"open_url","url":...}`).
  `match_kind` is unchanged.
- Use `id` as the stable identity (it is no longer a path). Open `launch`
  actions as today; open `open_url` actions with `NSWorkspace.open(URL)`.
  Show `subtitle` under the title.
- `record_selection` sends result `id`s in `shown` and `selected`.
- **Rust side wired 2026-10-08.** `milky_abi_version()` returns 3 and
  `milky.h` says 3. Verified from C against the real archive: v3 search and
  selection round trips, 1000 requests, 0 leaks. `record_selection` with a
  bare path now answers `bad_request` ("not a result id").
- **Swift side wired 2026-10-08.** The actor validates ABI 3, decodes `id`,
  `kind`, `title`, `subtitle`, and the tagged action into Swift-owned values
  before releasing Rust memory. App IDs must agree with their absolute launch
  path; setting URLs must have a scheme; duplicate or malformed IDs/actions
  are rejected. Native actions launch or activate app bundles and open settings
  deep links with `NSWorkspace.open`. Usage events report result IDs. Fixtures
  cover both kinds; fixture mode remains excluded from recording.
- The engine indexes 127 apps and 606 settings destinations; construction takes
  about 90 ms, so keep creating it off the main thread.


### Contract v4 — agreed 2026-10-08, Rust side wired

Authoritative text: shared [`../DECISIONS.md`](../DECISIONS.md), "Well-known
places, and contract v4". Swift-side summary:

- `milky_abi_version()` and `MILKY_ABI_VERSION` are now 4. Everything in v3 is
  unchanged; v4 only adds values.
- Result `kind` can also be `folder` (now: places such as Downloads, Trash,
  Applications) or `file` (not produced yet; file search, same contract).
- `action` can also be `{"type":"open","path":"/Users/x/Downloads"}`: open
  with `NSWorkspace.open(URL(fileURLWithPath:))`. Folders open in Finder; this
  is also how the Trash opens (verified: Finder shows its real Trash view).
- IDs: `folder:<absolute path>` and `file:<absolute path>`; the path must
  equal the `open` action's path, as `app:` IDs equal their launch path.
  `record_selection` accepts them.
- Example: `{"id":"folder:/Users/x/.Trash","kind":"folder","title":"Trash",
  "subtitle":"~/.Trash","action":{"type":"open","path":"/Users/x/.Trash"},
  "match_kind":"exact"}`.
- Presentation: the native file/folder icon for the path
  (`NSWorkspace.icon(forFile:)`), subtitle under the title as for apps.
- **Swift side wired 2026-10-08.** `ResultKind` accepts `folder` and `file`;
  the decoder accepts `open` only with an absolute path whose value matches the
  corresponding `folder:` or `file:` ID. `NativeAppOpener` uses
  `NSWorkspace.open(URL(fileURLWithPath:))`, icon lookup uses the same path,
  and selection recording continues to send the stable destination IDs.
  Real-engine tests cover Downloads, Applications, and Utilities; fixture
  snapshots cover the native folder icon. `file` is decoded and validated but
  no Rust file result is produced yet.

### Selection events — agreed and wired 2026-10-08

Authoritative text: shared [`../DECISIONS.md`](../DECISIONS.md), "Selection
events and durable usage history". Swift-side summary:

- After acting on a picked result, send one
  `{"op":"record_selection","query":...,"shown":[...],"selected":...,"outcome":"opened"|"failed"}`
  through the existing `milky_engine_request`. No new C functions.
- `shown` is every result ID in display order at the moment of the action;
  `selected` must be one of them. Send `failed` when opening fails. Only send
  after a real interaction.
- Expect `{"kind":"recorded"}`. A `bad_request` means the event was malformed
  and nothing was stored. Treat errors as non-fatal: never block or alter the
  launch, and do not retry in a loop.
- The call does durable disk writes, about 9 ms typically and 14 ms at p95. Do
  not issue it on the main thread.
- **Rust side wired 2026-10-08.** The engine validates, appends to
  `events.jsonl`, updates usage, and saves `usage.json` under one lock. Covered
  by Rust tests including 4 concurrent senders and a poisoned lock. Swift may
  start sending. The data directory is created on the first recorded event.
- Recorded launches reorder results immediately: usage breaks ties within a
  match kind (shared DECISIONS 2026-10-08). Result order still comes only from Rust.

- **Swift side wired 2026-10-08.** After each real action attempt, the launcher
  sends the query, displayed result IDs in order, selected ID, and `opened` or
  `failed` outcome through `RustSearchProvider`. Fixture mode has no recorder.
  The durable call runs asynchronously after the open attempt; errors are
  intentionally ignored and cannot delay opening or change the UI. The adapter
  accepts only `recorded` and surfaces malformed/error responses to the caller.

## Verified Swift adapter — 2026-09-13

- Real Rust search is the default. Only `--fixtures` selects development data;
  engine failures show errors with Retry and never trigger fixture fallback.
- `RustSearchProvider` holds its engine in an actor, prepares off the UI thread,
  serializes its own short synchronous calls, and skips cancelled queued work.
  The contract permits concurrency; this adapter does not require it. Existing
  presentation generations still discard obsolete responses.
- Every foreign response is copied into Data, freed with `defer`, then decoded.
  Result IDs, kinds, titles, subtitles, actions, and order come from Rust.
  Invalid app paths, malformed setting URLs, mismatched app actions, duplicate
  IDs, mismatched query echoes, unknown reply kinds, and invalid JSON fail
  visibly rather than becoming zero results.
- App termination cancels presentation work and awaits actor shutdown before
  replying to AppKit's termination request. Engine destruction cannot overlap
  its synchronous request. A shut-down provider cannot recreate an engine.
- Native launcher links the same Rust archive even in fixture mode, so the
  fixture launch wrapper now also builds Rust (without calling it for searches).
- Verified real Visual Studio Code and three IDLE results in the running app,
  real no-match response, and Enter launching Calculator. Swift bridge tests
  cover ABI 3, actual engine requests, app and setting decoding, action
  validation, post-free result lifetime, concurrent callers, escaped input,
  wire errors, unsupported versions, and shutdown.
- Selection messages and durable usage persistence were added on 2026-10-08;
  the adapter implementation is documented above. App discovery is still the
  Rust engine's startup snapshot; restart Milky to pick up installed/removed apps.
