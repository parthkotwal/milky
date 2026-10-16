# Architecture

## Product

A fully local macOS search and action engine intended to become a better personal replacement for Spotlight.

The system combines:

- native launcher behavior;
- heterogeneous local search;
- semantic and multimodal retrieval;
- personalized ranking;
- direct actions;
- background indexing;
- local-only ML inference.

## High-level design

```text
macOS UI
  Swift / SwiftUI / AppKit
        |
        v
Milky Core Service
  Rust
        |
        +--> Query understanding
        +--> Candidate generation
        |      - apps/actions
        |      - lexical search
        |      - semantic text search
        |      - image search
        |
        +--> Feature computation
        +--> Unified ranking
        +--> Action execution interface
        |
        +--> Metadata/history store
        +--> Search indexes
        +--> Local model runtime
```

A separate Python workspace is used for model experiments, offline evaluation, training, and export. Python should not be required for normal launcher runtime unless we later have a strong reason.

## Built so far

As of 2026-10-13. `milky-core` is the engine, `milky-ffi` exposes it to Swift
through five C functions (ABI 4, JSON messages; DECISIONS 2026-09-13 and later
contract entries), and `milky-cli` (`milky`) drives it headlessly: one-shot or
interactive queries, `--settings`, `--files`. 230 core and 8 FFI tests.

Sources, each turned into candidates by `candidate.rs` (one per destination,
with an ID, title, subtitle, and action):

- `apps.rs`: app bundles in `/Applications`, `/System/Applications`,
  `~/Applications` and one level below, named as Finder names them
  (`launch_services.rs`, the one home of the CoreFoundation calls).
- `places.rs`: 13 well-known folders (Downloads, Trash, Applications,
  iCloud Drive, ...), with aliases that match but are never shown.
- `settings.rs`: System Settings panes and their sections with Apple's
  keywords, opened by deep link.
- `files.rs`: the home folder and iCloud Drive as Finder shows them, minus
  generated folders and the user's `exclusions.txt`.

Here: 127 apps, 13 places, 606 settings, ~5k files and folders. `Engine::new`
finds apps, places, and settings (~0.11 s) and publishes them as the first
`Index` snapshot; a background thread (`indexer.rs`) walks the files and
publishes them (~0.18 s here), then follows FSEvents: events outside indexed
folders are dropped, each ~300 ms burst re-reads only the folders that
changed, and apps are re-discovered when `/Applications` changes. Searches
clone the current `Arc<Index>` and never wait for the indexer.

A query (`search.rs`, ~0.2-0.35 ms here):

1. `normalize_query` folds case, accents, and dashes and turns other
   punctuation into spaces; names go through the same function.
2. Each candidate is matched (`matching.rs`): titles by an ordered
   `MatchKind` ladder over precomputed keys (words split at spaces,
   punctuation, letter/digit and camelCase boundaries), settings also by
   keywords, files by `file_tier` (a strong match on the name, folders and the
   extension only supporting it; the whole name or a pasted path is exact).
3. Hits are ordered by `rank`: standing (the tier, two steps lower for
   settings sections and files), then usage (`usage.rs`, decayed selection
   counts), then prior (app > place > pane > section > folder > file), then
   recency for files.
4. `select_best` keeps the top `limit` without a full sort, at most 40% files
   unless nothing else fills the list; `with_typed_folder` adds the folder the
   query named on the way to a file.

Interactions are stored in `milky.sqlite3` (`store.rs`) in
`~/Library/Application Support/Milky/`: every search gets an ID and is kept in
memory with its results, its top 50 candidates, and their features; when the
launcher reports a session (`record_events`: query states, drawn searches,
highlights, actions, how it ended, context), the drawn searches are stored
with it and picks update `usage.json` (decayed scores that break ranking
ties). The store also holds app focus intervals (`record_focus`), dwell after
picks, and per-folder change counts from the indexer; served files carry when
they were last opened (macOS's `com.apple.lastuseddate#PS` attribute) and how
busy their folders are (`activity.rs`). `milky --export`, `--pause-history`,
and `--forget-history` manage it; `MILKY_DATA_DIR` points all of this
elsewhere, for tests.

## Runtime components

### macOS client

Responsibilities:

- global launcher shortcut;
- query input;
- result list;
- previews;
- keyboard navigation;
- action menus;
- native macOS integrations.

The UI should stay thin. Search and ranking logic belongs in the core service.

### Rust core

Responsibilities:

- query processing;
- candidate generation;
- lexical retrieval;
- semantic retrieval;
- image retrieval;
- feature generation;
- ranking;
- metadata access;
- filesystem event processing;
- indexing queues;
- caches;
- communication with local model runtimes;
- performance-sensitive work.

The current integration uses an in-process Rust static library, a handwritten
C header, and a SwiftPM system-library target. The engine/search contract (DECISIONS
2026-09-13) is implemented, now at ABI version 4 (contract v4): five C functions, JSON
request/response messages through `Engine::handle_json`, and concurrent calls
allowed. Swift search and per-session interaction-event adapters use the same
boundary. Rust stores the session timeline and search impressions locally.
Swift-side coordination lives in
`.agents/swift/INTEGRATION.md`; cross-language decisions remain shared.

### Background indexer

The indexer should eventually:

- discover searchable sources;
- extract metadata;
- extract supported document text;
- generate embeddings;
- watch filesystem changes;
- update indexes incrementally;
- remove stale entries;
- schedule expensive work without hurting foreground latency.

### Python ML workspace

Used for:

- evaluating embedding models;
- ranking experiments;
- offline metrics;
- training learned rankers;
- personalization experiments;
- model conversion/export;
- benchmark notebooks/scripts.

Runtime models should be exported into a format appropriate for efficient local inference.

## Search pipeline

Target shape:

```text
query
  |
  v
query normalization / intent features
  |
  +------------------------------+
  |              |               |
  v              v               v
structured     lexical        semantic
sources        retrieval      retrieval
(apps,         (files/text)   (text/images)
actions)
  |              |               |
  +--------------+---------------+
                 |
                 v
          candidate merge
                 |
                 v
          feature computation
                 |
                 v
          personalized ranker
                 |
                 v
             results
                 |
                 v
              actions
```

Easy queries should take a cheap path. Expensive models should only run when they add value.

## Initial searchable sources

V1 target:

- installed applications;
- files and folders;
- supported document contents;
- screenshots and images;
- macOS settings and system actions;
- web-search fallback.

Later:

- browser history;
- notes;
- clipboard history;
- cloud files;
- contacts;
- other personal sources where local access is appropriate.

## Ranking

Initial ranking can combine explicit features such as:

- exact/prefix/fuzzy lexical match;
- BM25-like score;
- semantic similarity;
- candidate type;
- recency;
- frequency;
- prior query-to-selection behavior;
- overall selection frequency;
- current foreground application;
- basic query intent.

Selections and result impressions should be stored from the beginning so later learned ranking is possible.

Likely evolution:

1. hand-tuned scoring;
2. offline learned ranker;
3. query-specific personalization;
4. contextual ranking;
5. online/adaptive personalization where appropriate.

## Multimodal search

Screenshots and images are a first-class target.

Initial representation may combine:

- metadata;
- OCR text;
- image embeddings.

Example intended queries:

- `screenshot of the seiko watch`
- `diagram with kafka boxes`
- `restaurant menu photo`
- `brown jacket screenshot`

Model choice is intentionally undecided until benchmarking.

## Storage

Tentative split:

- SQLite: metadata, usage history, configuration, durable structured state;
- dedicated lexical index;
- dedicated vector index;
- local model files and caches.

The project should not force all data through one storage abstraction.

## Performance

Foreground search should feel instantaneous.

Important principles:

- keep the search service warm;
- return cheap high-confidence results early;
- refine results progressively when useful;
- cache common work;
- incrementally update indexes;
- benchmark before optimizing;
- move hot paths lower in the stack only when measurements justify it.

Potential future work includes:

- memory-mapped indexes;
- compact binary formats;
- quantized embeddings;
- SIMD;
- custom ANN structures;
- Metal / Apple Neural Engine acceleration;
- specialized native inference.

## Entity graph

A graph is a likely later subsystem, not a V1 requirement.

Purpose:

- connect files, folders, projects, apps, URLs, courses, people, and other entities;
- allow related objects to influence retrieval and ranking;
- expose useful actions around a conceptual entity rather than only raw files.

Possible future techniques include graph traversal, personalized PageRank-like features, graph embeddings, and learned relationship discovery.

## Fully local constraint

Core behavior must work without a network connection.

User content, embeddings, query history, selections, and personalization stay on the device.

This constraint is intentional and should influence model size, inference strategy, storage, caching, and indexing design.

## Open architectural decisions

Decided so far: the phase-1 interaction event schema (DECISIONS 2026-10-15), the
filesystem discovery policy (2026-10-08), and app, places, and System Settings
sources. Still to decide:

- how the file index is built off the startup path and kept fresh (threads,
  FSEvents), and whether it is persisted;
- post-pick dwell, focus intervals, retention/export, and privacy controls for
  interaction history;
- exact lexical index implementation for document contents (an inverted
  index; names are scanned linearly at ~5k);
- exact vector index implementation;
- local inference runtime;
- initial text embedding model;
- initial image embedding model;
- document parsers and supported formats;
- ranking feature schema and learned ranker (hand-set order today);
- system actions (sleep, lock, empty trash).
