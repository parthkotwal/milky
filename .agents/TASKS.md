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
- [x] Swift adapter against the contract (Swift owner), 2026-09-13.
- [x] Swift binaries keep a stale Rust archive after Rust rebuilds. Worked
      around by `apps/macos/scripts/build.sh` (Swift owner); bare `swift build`
      still bypasses it.
- [x] Usage persistence: `storage.rs` (atomic write, durable append),
      `usage.json` with corrupt-file quarantine, `events.jsonl` log. 2026-10-08.
- [x] `record_selection` message, wired through `Engine` under a `Mutex`
      (DECISIONS 2026-10-08). 103 Rust tests.
- [x] Swift sends `record_selection` after real open attempts (Swift owner,
      2026-10-08; verified in native tests and real bridge contract tests).
- [ ] Swift adapter on contract v3: decode `id`/`kind`/`title`/`subtitle`/`action`,
      open URLs, send result IDs (Swift owner). See `.agents/swift/INTEGRATION.md`.
- [x] Rank with usage: break ties within a match kind by decayed launch score
      (DECISIONS 2026-10-08). 107 Rust tests.
- [ ] Offline ranking evaluation over `events.jsonl` once real selections exist:
      replay logged queries, compare rankers by where the pick lands (e.g. MRR).
- [x] Define the common candidate/result/action data model: `candidate.rs`,
      destination IDs, `Action` (launch / open URL), contract v3 (DECISIONS 2026-10-08).
- [x] Install full Xcode. Resolved in ISSUES.md; verify the active developer
      directory on a fresh machine.
- [x] Define the selection event schema (DECISIONS 2026-10-08). Dismissal
      without selection is not recorded yet.
- [ ] Choose the first lexical retrieval implementation. App-name matching is a
      linear scan over 128 bundles at ~30-90 us, which is fine at this size and
      will not be for files.
- [ ] Rank with recorded usage so it can distinguish equally-good prefix matches.
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
- [x] Results carry a subtitle; the three `IDLE.app`s read "Python 3.10/3.11/3.12"
      (contract v3, 2026-10-08).
- [x] camelCase word boundaries: `ColorSync` yields words and initials `c`, `s`;
      whole-word initials still count (2026-10-08).
- [x] Unicode: queries and names fold accents, compatibility forms, and invisible
      marks (`cafe` matches `café`). Not full case folding: `ß` stays `ß`.
- [ ] Benchmark candidate local text embedding models.
- [ ] Benchmark candidate local image embedding models.
- [ ] Decide local model inference runtime.
- [ ] Define initial indexing policy and supported document types.
- [ ] Design the first unified ranking feature set.
- [ ] Define latency targets and a benchmark harness.

## Soon

- [x] App discovery and launching, with Finder's display names (2026-10-08).
- [x] Display names come from LaunchServices, called from Rust (DECISIONS 2026-10-08).
- [ ] File and folder search. Plan agreed 2026-10-08. Goal: a few letters of
      a file or folder put the right one at or near the top, including basic
      places (Applications, Downloads, Documents), where Spotlight fails.
      Rules are general conventions or user settings, never tuned to one Mac.
      Measured: home folder 3.2M entries / 18 s raw; a Finder-style walk finds
      ~23k entries in ~55 ms (warm). Linear name scan ~1 ms at that size; an
      inverted index waits for document contents.
      1. [x] Walk (2026-10-08, `files.rs`, `milky --files`): home + iCloud Drive; skip what Finder hides (dot names,
         `UF_HIDDEN`, which covers `~/Library`); packages are one result; no
         symlinks; unreadable folders reported. Exclusion list in Milky's data
         folder, user-editable, created with defaults for known tool output
         (`node_modules`, `__pycache__`, Go's module cache, ...); folders
         marked `CACHEDIR.TAG` / `pyvenv.cfg` / `conda-meta` skipped. Project
         source files are indexed. Measured on this Mac: 22,875 entries
         (3,174 folders), 0.30 s first walk, 0.05 s warm; without the defaults,
         Go's module cache alone added ~36,800.
      2. [ ] Well-known places as results: Home, Desktop, Documents,
         Downloads, Applications (`/Applications`, system), Utilities, Movies,
         Music, Pictures, Public, iCloud Drive, Library, Trash. Names from
         macOS (localized). Rank high: `downloads` -> Downloads first.
      3. [ ] Files in search: path-aware matching (`332 ex01` ->
         `school/cse332/ex01.pdf`), file ranking (match, usage, recency,
         location, folder vs file) tuned on a query set; top-k selection.
      4. [ ] Background indexing: startup unchanged, files arrive after.
      5. [ ] Launcher: contract v4 (file/folder/place kinds, open and reveal
         actions), folder-permission prompts. Rebuild the launcher here.
      6. [ ] Freshness via FSEvents.
      Later, opt-in: items inside the Trash. `~/.Trash` is unreadable without
      Full Disk Access ("Operation not permitted", even from Terminal), so it
      waits until Milky has another reason to ask for that permission. Shown as
      the item itself, subtitled "In Trash", revealed in Finder.
- [ ] Document text extraction.
- [ ] Screenshot OCR.
- [ ] Image embedding index.
- [ ] Semantic text retrieval.
- [ ] System Settings source: panes *and the sections inside them*, the way
      Spotlight finds "Bluetooth", "Night Shift", or "Camera" (privacy). Raised by
      the user 2026-10-08 as critical for daily use; agreed to come before files.
      Measured 2026-10-08 on macOS 26:
      - 51 panes in `/System/Library/ExtensionKit/Extensions` with extension
        point `com.apple.Settings.extension.ui`. `Info.plist` →
        `EXAppExtensionAttributes.SettingsExtensionAttributes` carries
        `allowsXAppleSystemPreferencesURLScheme`, `searchTermsFileName`,
        `legacyBundleIdentifier`, and sometimes `presentsInSidebar: false`.
      - 47 panes ship `en.lproj/<name>.searchTerms` (XML plist): 563 sections
        keyed by anchor, 711 titled entries, 3,520 keywords. Anchors match
        deep-link anchors (`Privacy_Camera`). "Night Shift" →
        `nightShiftSection` in Displays.
      - Pane names: neither `Info.plist` nor LaunchServices is right for all
        (LaunchServices returns `Date & Time.appex` but also `DisplaysExt.appex`).
        Sidebar names are localization keys (`ENERGY_SAVER_PREF_TITLE`), some
        hardware-dependent (Battery vs Energy). Unresolved.
      - Verified 2026-10-08 on macOS 26, by screenshot after each `open`:
        `x-apple.systempreferences:<pane bundle id>?<searchTerms anchor>` opens
        the exact section. The pane's own `CFBundleIdentifier` works; the
        legacy ID is not needed. `com.apple.preference.security?Privacy_Camera`
        → Camera list; `com.apple.Displays-Settings.extension?nightShiftSection`
        → Night Shift panel; `com.apple.wifi-settings-extension?Advanced` →
        Advanced panel. Some sections open as a sheet over the pane. A link sent
        while a sheet is closing can be ignored.
      - Identity: resolved 2026-10-08. Destination IDs (contract v3); usage
        history keyed by path migrates to `app:<path>` on load.
      - Punctuation: Apple writes "Wi‑Fi" with U+2011. `wifi` only matches as a
        subsequence and `wi-fi` not at all; hyphens need folding.
      Step 1 done 2026-10-08 (`settings.rs`, `milky --settings`): 51 panes,
      711 items. Names: hardware-dependent sidebar key (power source checked via
      IOKit; Touch ID and Apple Intelligence assumed present), else
      `InfoPlist.loctable` `en`, else `Info.plist`, else cleaned identifier.
      ~40 ms warm at startup (Info.plist scan 16 ms, loctables 19 ms, terms
      6 ms); cacheable by OS version if it ever matters. Not yet searchable.
      Known gaps: "AutoFill & Passwords" (shown under General) is not a pane and
      is not in any search-terms file; General itself has no items; panes hidden
      by hardware (Mouse without a mouse, Classroom) are still listed;
      Accessibility is 342 of 711 items, so keyword matches must not flood
      results; English only.
      Searchable since 2026-10-08 (contract v3): pane names and section titles.
      Step 3 done 2026-10-08: keyword matching, multi-word queries, tiered
      ranking with an app > pane > section prior, subsequence for apps only
      (DECISIONS 2026-10-08). Remaining misses: `location`, `display`,
      plurals (`hot corners`), `screen recording`.
- [ ] System actions (sleep, lock, empty trash, etc.).
- [ ] Web-search fallback.
- [x] Usage-history persistence.
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
