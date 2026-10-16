# Tasks

This is the shared work queue. Bugs and lessons belong in ISSUES.md; native UI
implementation details belong in `.agents/swift/TASKS.md`. History and
measurements belong in DECISIONS.md; entries here point there.

Keep entries concise. Remove or move completed items when they no longer help future sessions.

## Now

- [ ] Document search (DECISIONS 2026-10-16): extraction with Apple's
      engines into a text cache, our own inverted index with BM25 and prefix
      match, content results ranked below name matches (at most 3, all words
      required) with snippets; OCR last; FTS5 as a test oracle. Later stage:
      a compact memory-mapped on-disk index.
      D1-D4 done 2026-10-16 (extraction, text cache, word splitting, the
      inverted index with prefix match, BM25, phrases, top-k). Next: D5
      (Claude): content worker, search integration, snippets, FTS5 oracle.
      Measured: index ~142 MB in RAM, 2.2 s to build at startup, so the
      compact format is due (DECISIONS 2026-10-16, D3/D4 notes).
- [ ] Interaction logging, phase 1 (DECISIONS 2026-10-15). Rust done
      2026-10-16: SQLite store, `search_id`, impressions with features,
      `record_events`, `events.jsonl` imported, `milky --log`. Swift (in
      progress): session timeline and context sent as `record_events`, then
      stop sending `record_selection`; set `MILKY_DATA_DIR` in tests that
      create a real engine.
- [ ] Interaction logging, phase 2 (Rust done 2026-10-16: last-opened dates,
      folder activity, `record_focus`, `dwell` events). Swift: dwell after a
      pick and the focus-interval observer with idle/sleep/lock detection
      (`swift/INTEGRATION.md`, phase 2).
- [ ] Interaction logging, phase 3: export, pause, and delete are done
      (`milky --export`, `--pause-history`, `--forget-history`). Retention
      must land before the first sessions turn 90 days old (about
      2027-01-14): keep full detail 90 days, then compact (design: which
      summaries keep their value). Pause and delete in the launcher's UI
      (Swift, later).

## Soon

- [ ] Offline ranking evaluation: replay `events.jsonl` queries against the
      current engine and report where the pick lands (top-1, MRR), so every
      ranking change is measured on real use. First data, 2026-10-13: 12 of 15
      picks were first; the misses were `applications ` (pick 4th), `system`
      (2nd), and `chrome` (2nd; the `.localized` bug, since fixed).
- [ ] Learn the ranking from selections. The hand-set order has hit its limit
      (DECISIONS 2026-10-12): no single ladder ranks a file's exact name above
      a long setting starting with the word, an app above a file named after
      one of its words, and a pane above an app starting with its name.
      `TAIL_STEPS` is the current compromise; usage cannot lift a section or
      file over an app of equal strength. Needs the evaluation above and more
      selections first.
- [ ] Folder activity as a ranking feature (raised 2026-10-14): a decayed
      per-folder count of file changes, rolled up to the project root (a
      folder with `.git`, `Cargo.toml`, `package.json`, ...), so files in the
      area being worked on rank higher among equals (`readme` -> the active
      project's README). Machine churn (agents, builds, sync) says the project
      is active, not that its files are wanted: cap bursts, never count a file
      more relevant for being rewritten. A feature for the learned ranker, not
      a hand-set rule; the live index's watcher can count it cheaply.
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
- [ ] Tell the user about unreadable folders. `Walk::unreadable` lists folders
      macOS privacy settings blocked, but the engine does not report them, so
      a denied Desktop/Documents/Downloads prompt silently hides files.
- [ ] Screenshot OCR; image embedding index; semantic text retrieval. Benchmark
      local text and image embedding models and choose an inference runtime
      first.
- [ ] System actions (sleep, lock, empty trash) and a web-search fallback.
- [ ] Latency targets and a benchmark harness. Today: queries 0.2-0.35 ms with
      ~5.7k candidates; the FFI concurrency test (1,600 searches) is the best
      slowdown canary.
- [ ] Known search gaps:
      - settings: `location`, `display`, plurals (`hot corners` vs keyword
        `hot corner`), `screen recording`; "AutoFill & Passwords" is in no
        search-terms file; panes hidden by hardware are still listed; English
        only;
      - files: `final report` is a word-prefix (not prefix) match for
        `final-report`, because folding drops dashes.
- [ ] Indexer publish cost: each update rebuilds every candidate (~33 ms for
      ~5k files here), at most once per 300 ms batch. Keep file candidates
      between updates and rebuild only changed entries if file counts grow.
- [ ] Small: cache folder words per folder when building file candidates
      (~140 ms for 23k entries, recomputed per file); `milky` panics on a
      closed stdout (`milky --files | head`).

## Later

- [ ] User settings for result mix: the file cap (now 40% of the list) and
      possibly the kind order. Requested 2026-10-08; `search::file_cap` is the
      single place to change.
- [ ] User settings for which folders are indexed: today the home folder and
      iCloud Drive minus `exclusions.txt`, edited by hand. Requested 2026-10-09.
- [ ] Items inside the Trash, opt-in: `~/.Trash` is unreadable without Full
      Disk Access ("Operation not permitted", even from Terminal). Show the item
      itself, subtitled "In Trash", revealed in Finder.
- [ ] Contextual personalization.
- [ ] Browser history.
- [ ] Entity graph.
- [ ] Quantized embeddings.
- [ ] Custom or specialized search indexes where justified.
- [ ] Native performance work based on profiling.
- [ ] Broader personal data sources.

## Done

Milestones, newest first. Details live in the DECISIONS entries named.

- Live file index: snapshots, a background indexer, FSEvents-driven updates
  of files and apps; `milky --watch` (2026-10-14).

- Swift records Open, Reveal, and Copy outcomes through the existing selection
  message and logs recording failures locally (2026-10-13; Swift decision).
- Ranking: head and tail (settings sections and files two steps below apps,
  places, panes); `.localized` folders by Finder name (2026-10-12).
- File search refinements from daily use: queries split like names, whole-name
  and pasted-path matches are exact, short-query rule, the typed folder offered
  last (2026-10-09).
- File and folder search: Finder-visible walk with user exclusions; strong name
  match with folder/extension support; file cap; recency; top-k selection
  (2026-10-08/09).
- Well-known places and contract v4 (folder/file kinds, `open` action); Swift
  adapter on v4 (2026-10-08).
- System Settings as a source: panes and sections, keywords, deep links
  (2026-10-08).
- Results carry a kind, an action, and a destination ID (contract v3); usage
  keyed by ID with migration (2026-10-08).
- Selection events and durable usage history; usage breaks ties (2026-10-08).
- App discovery with Finder's display names, folded and camelCase-aware
  matching (2026-09-11, 2026-10-08).
- Swift ↔ Rust boundary: five-function C ABI with JSON messages (2026-09-13).
- Repository structure, separate Swift agent guidance, full Xcode (2026-09).
