# Tasks

This is the shared work queue. Bugs and lessons belong in ISSUES.md; native UI
implementation details belong in `.agents/swift/TASKS.md`.

Keep entries concise. Remove or move completed items when they no longer help future sessions.

## Now

- [x] Prepare separate Swift agent guidance, design direction, and local records.
      Native implementation work is tracked in `.agents/swift/TASKS.md`.

- [x] Define repository structure for Swift app, Rust core, and Python ML workspace.
      `rust/` workspace with `milky-core`, `milky-ffi`, `milky-cli`; `apps/macos`
      SwiftPM package.
- [x] Decide Swift ↔ Rust integration approach. Rust `staticlib` linked into the
      app, unmangled `extern "C"` symbols, hand-written header wrapped by a
      SwiftPM `systemLibrary` target. Proven by `milky-probe`; only
      `milky_abi_version` crosses so far.
- [x] Implement the Rust side of the Swift ↔ Rust boundary (DECISIONS
      2026-09-13): `api.rs` message types, `Engine::handle_json`, five
      `milky-ffi` exports, header at ABI version 2. Verified: 75 Rust tests, a C
      program making 1000 requests through the real archive, `leaks` 0 leaks.
- [ ] Swift adapter against the contract (Swift owner).
- [ ] Swift binaries keep a stale Rust archive after Rust rebuilds; see
      ISSUES.md 2026-09-13. Needs a durable build fix (Swift owner).
- [ ] Usage persistence (serde, `UsageError`, atomic save). Specced 2026-09-11,
      not written. Next natural consumer: selection events from the launcher.
- [ ] Define the common candidate/result/action data model.
- [x] Install full Xcode. Resolved in ISSUES.md; verify the active developer
      directory on a fresh machine.
- [ ] Define the query and selection event schema.
- [ ] Choose the first lexical retrieval implementation. App-name matching is a
      linear scan over 128 bundles at ~30-90 us, which is fine at this size and
      will not be for files.
- [ ] Record selections so ranking can use usage. Nothing distinguishes ten
      equally-good prefix matches today.
- [ ] Usage is three separate features, not one number. Keep them separate so
      each can be weighted and debugged independently:
      1. invocation frecency — decayed launch count, in `usage.rs`;
      2. currently running — one OS query; 5 of 128 indexed apps on this
         machine, so it is high-precision. Read it via `NSWorkspace` on the
         Swift side and pass it into the engine as input, the way `now` is
         passed in. Launch count alone undercounts apps left open for days.
      3. focus time — foreground duration per app. The best signal and the
         most expensive: needs a resident observer on
         `NSWorkspace.didActivateApplicationNotification`. Note open is not
         used: 45 bundles run on this machine while about three are in use.
- [ ] A running app's action is "switch to", not "launch". Affects the result
      model, not just ranking.
- [ ] Results need a subtitle: three Python installs each ship `IDLE.app`, so
      the list shows the same word three times.
- [ ] camelCase word boundaries: `ColorSync` should yield initials `c` and `s`.
- [ ] Unicode: `to_lowercase` is not case folding and ignores accents, so `cafe`
      will not match `café`.
- [ ] Benchmark candidate local text embedding models.
- [ ] Benchmark candidate local image embedding models.
- [ ] Decide local model inference runtime.
- [ ] Define initial indexing policy and supported document types.
- [ ] Design the first unified ranking feature set.
- [ ] Define latency targets and a benchmark harness.

## Soon

- [ ] App discovery and launching. Bundle scanning works; display names are still
      the bundle directory name, which is wrong for some apps (see `ISSUES.md`).
- [ ] Decide how to get correct localized app display names: `core-foundation`
      from Rust, or supplied by Swift.
- [ ] File/folder indexing.
- [ ] Document text extraction.
- [ ] Screenshot OCR.
- [ ] Image embedding index.
- [ ] Semantic text retrieval.
- [ ] System settings/actions source.
- [ ] Web-search fallback.
- [ ] Usage-history persistence.
- [ ] Background filesystem watching and incremental updates.
- [ ] Result previews and secondary actions.

## Later

- [ ] Learned ranking.
- [ ] Contextual personalization.
- [ ] Browser history.
- [ ] Entity graph.
- [ ] Quantized embeddings.
- [ ] Custom or specialized search indexes where justified.
- [ ] Native performance work based on profiling.
- [ ] Broader personal data sources.
