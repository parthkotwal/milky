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

`milky-core` holds the engine: `normalize_query`, app bundle discovery under
`/Applications`, `/System/Applications`, `~/Applications` and one level of
subdirectories, a `MatchKind` ladder, and an `Engine` that scans once and
answers queries. `milky-cli` (`milky`) drives it headlessly, including an
interactive mode against one warm engine. `milky-ffi` exposes the
five-function ABI; `api.rs` holds the wire types. `record_selection` messages
append to `events.jsonl` and update `usage.json` (decayed launch scores) under a
`Mutex` in the engine, in `~/Library/Application Support/Milky/`. Usage breaks
ties within a match kind; it never outranks a stronger match.
`settings.rs` reads System Settings panes and their sections (51 panes, 711
items, deep links verified). `places.rs` contributes existing well-known
folders such as Downloads, Trash, Applications, and Utilities. `candidate.rs`
turns 127 apps, 13 places, and 606 settings into one list of destinations with
IDs, subtitles, and actions;
`files.rs` walks the home folder and iCloud Drive with Finder's visibility
rules and a user-editable exclusion list (~23k entries here, 0.05-0.3 s); file
results are not wired into search yet. Search ranks apps, places, and settings
together by tier (title match kind interleaved with Apple's keyword matches),
then usage, then destination prior (app > place > pane > section). Apps carry
Finder's display names and a precomputed matching key (folded,
split on whitespace and camelCase). Measured: ~90 ms to index 127 apps and 606
settings destinations, ~75-90 us per query (keywords scanned linearly; an
inverted index waits for file search).

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
allowed. Swift search and selection-event adapters use the same boundary.
Selection events are stored locally by the Rust engine. Swift-side coordination lives in
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

Still to decide:

- Swift ↔ Rust event message schema (impressions, selections, action outcomes);
- exact lexical index implementation;
- exact vector index implementation;
- local inference runtime;
- initial text embedding model;
- initial image embedding model;
- document parsers and supported formats;
- filesystem discovery/indexing policy;
- ranking feature schema;
- app/system-action source implementation.
