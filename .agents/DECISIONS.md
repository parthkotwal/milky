# Decisions

Keep durable product and architecture decisions here.

Use this format:

```text
## YYYY-MM-DD — Decision title

Decision:
...

Why:
...

Alternatives considered:
...

Consequences:
...
```

---

## 2026-09-07 — Fully local product

Decision:

Core search, indexing, inference, usage history, and personalization will run locally on the Mac.

Why:

Privacy is part of the product identity, offline behavior matters for a launcher, and local inference creates interesting engineering problems around latency, memory, storage, and model efficiency.

Consequences:

Cloud-only models cannot be required for core behavior. Model and index choices must be evaluated on Apple hardware.

---

## 2026-09-07 — Hybrid search and action interface

Decision:

The launcher will unify search results and executable actions rather than behave only as a document finder.

Why:

The intended product should replace both Spotlight-like search and common launcher/action workflows.

Consequences:

The internal result model needs to represent candidate entities, result types, and supported actions.

---

## 2026-09-07 — Initial source scope

Decision:

The first serious version targets applications, files/folders, document contents, screenshots/images, system actions/settings, and web-search fallback.

Broader personal sources come later.

Why:

This is broad enough to express the product thesis while avoiding early integration sprawl.

---

## 2026-09-07 — Initial language split

Decision:

Use Swift/SwiftUI/AppKit for the macOS interface, Rust for the core engine, and Python for ML experimentation.

Why:

Swift provides native macOS integration, Rust gives the core a strong systems/performance path, and Python keeps ML experimentation flexible.

Consequences:

We need a clean Swift/Rust boundary and a model-export path that does not require Python in normal runtime.

---

## 2026-09-07 — Learn from selections

Decision:

Record result impressions and selections locally from early versions.

Why:

Personalized ranking is a core long-term capability, and useful interaction data cannot be reconstructed later if it was never recorded.

Consequences:

The event schema should be designed before the first real dogfooding period.

---

## 2026-09-11 — Match quality is an ordered kind, not a score

Decision:

`matching::MatchKind` is an enum ordered weakest to strongest — `Subsequence`,
`Substring`, `Acronym`, `WordPrefix`, `Prefix`, `Exact` — and `match_kind`
returns the strongest that applies. Ranking sorts by kind, then by name length,
then alphabetically.

Why:

Assigning numbers to match quality invites invented precision: nobody can say
whether a prefix match is worth 0.8 or 0.75. An ordered enum states only what was
actually observed. It is also totally ordered, so sorting is exact and avoids
float comparison entirely.

The length tie-break exists because ties are common — every result for a
one-character query is a prefix match — and an arbitrary order makes results
jitter between keystrokes, which is a correctness problem, not a cosmetic one.

Consequences:

This ladder cannot distinguish candidates of the same kind by anything other
than name. Ranking `Chess` above `Calendar` for `c` is impossible here and needs
a usage signal. When that arrives, kind becomes one feature among several and
this enum turns into an input to scoring rather than the ranking itself.

Acronym matching uses `word_initials(name).starts_with(query)`, so a query may
cover the leading words rather than all of them. Splitting on whitespace only
means `ColorSync` yields one initial, not two; camelCase boundaries are a known
gap.

---

## 2026-09-11 — Scan installed apps once and hold them

Decision:

`Engine` scans for app bundles at construction and keeps the result. `search`
takes `&self`; `reindex` takes `&mut self` and replaces the list.

Why:

A full scan measures ~1.6 ms. A keystroke budget is one 16 ms frame shared with
every other source, ranking, and drawing, so re-scanning per query is out.

`&self` on `search` means concurrent queries remain possible without redesign;
`&mut self` there would serialize every search behind an exclusive borrow. When
something inside eventually needs to mutate, the answer is interior mutability,
not changing this signature.

Consequences:

The index goes stale until `reindex` is called. A filesystem watcher is the
eventual trigger.

---

## 2026-09-11 — Separate Swift agent guidance and memory

Decision:

Keep native design, working guidelines, tasks, decisions, issues, and integration
notes in `.agents/swift/`. Root and macOS `AGENTS.md` entry points route agents
to the shared instructions and the relevant Swift guidance.

Why:

The user wants Swift agents to develop the native experience and retain their
own context without interfering with the core agent's learning and work records.

Alternatives considered:

Putting every UI detail in shared memory would mix independent work queues.
Entirely separate instructions would risk divergent product and FFI decisions.

Consequences:

Swift-only work updates local records. Product-wide and cross-language changes
remain coordinated and recorded here. The Swift integration file distinguishes
verified behavior from proposals, allowing fixture-based UI work before the real
search boundary is ready. No application behavior is implemented by this setup.

---

## 2026-09-13 — Swift ↔ Rust boundary: message-based JSON over a five-function C ABI

Status: accepted by the user (core owner's call, delegated). Rust side implemented
2026-09-13; Swift adapter connected 2026-09-13.

Decision:

ABI version 2. The C surface is five functions and does not grow with features:

```c
typedef struct MilkyEngine MilkyEngine;              /* opaque */

uint32_t     milky_abi_version(void);
MilkyEngine *milky_engine_new(void);                  /* NULL on failure */
void         milky_engine_free(MilkyEngine *engine);  /* NULL-safe */
char        *milky_engine_request(const MilkyEngine *engine,
                                  const char *request_json); /* owned, never NULL */
void         milky_string_free(char *s);              /* NULL-safe */
```

Requests and responses are UTF-8 JSON. The schema is serde types in `milky-core`,
mirrored by Codable types in Swift. The first message is search:

```text
request   {"op":"search","query":"term","limit":20}
response  {"kind":"search","query":"term","results":[
             {"path":"/System/Applications/Utilities/Terminal.app",
              "name":"Terminal","match_kind":"prefix"}]}
error     {"kind":"error","reason":"bad_request","message":"..."}
          reason: bad_request | internal | panic
```

Why:

The result model is undesigned and will grow: files, actions, subtitles, a
"switch to" verb for running apps, score breakdowns. With `#[repr(C)]` structs,
every field is a header change, an ABI bump, and a Swift adapter change,
coordinated between two agents. A hand-written header that disagrees with Rust
corrupts memory silently; a schema mismatch fails as a decode error. More
operations are coming (selection events, reindex), and indexing may move out of
process, where a message API carries over directly and a struct ABI does not.

Measured cost, 20 results, release builds:

| step | JSON | C structs |
|---|---|---|
| Rust encode | 1.2 us (serde_json) | 1.1 us (CString) |
| Swift decode | 23.4 us (JSONDecoder) | 1.4 us (String(cString:)) |

JSON marshaling is about 10x slower, roughly 25 us per keystroke, dominated by
Swift's decoder. That is under 0.2% of a 16 ms keystroke budget and smaller than
the search itself (30-90 us).

Alternatives considered:

- `#[repr(C)]` result arrays: fastest, but rigid, and the largest unsafe surface.
- One JSON function per operation: a more descriptive header, but an ABI change
  and agent coordination for every new operation.
- A binary codec: faster, but dependencies on both sides, harder to debug, and
  premature at this payload size.

Revisit when: profiling shows decoding as a meaningful share of keystroke
latency, or payloads grow by orders of magnitude. The swap is a deliberate ABI
bump, not a gradual drift.

Consequences — the contract:

- Engine: `milky_engine_new` returns NULL on failure. `milky_engine_free` must
  never run while a request is in flight; Swift frees only at termination.
- Strings: the request is borrowed only for the duration of the call. The
  response is owned by the caller, never NULL, freed exactly once with
  `milky_string_free`, and does not depend on the engine staying alive.
- Errors are distinguishable from empty results: `kind: "error"` versus
  `kind: "search"` with `results: []`.
- Threading: concurrent `milky_engine_request` calls on one engine from any
  thread are allowed. `Engine` is `Send + Sync`; verified with 16 threads x 500
  overlapping searches. Required because the launcher starts a new search per
  keystroke without waiting for the previous synchronous call to return.
- Panics: every entry point wraps its body in `catch_unwind` and returns an
  error response, or NULL from `milky_engine_new`. Verified that a panic escaping
  `extern "C"` aborts the whole process (SIGABRT, exit 134).
- No cancellation in v1: a search costs 30-90 us, and Swift discards stale
  responses by generation.
- Match kind crosses as a snake_case string, never a number: `MatchKind`
  declaration order is the ranking order and will change.
- The header stays hand-written; Swift checks `milky_abi_version()` at startup.

---

## 2026-10-08 — Selection events and durable usage history

Status: accepted (core owner's call, delegated by the user). Rust message types,
storage, log, and engine wiring implemented 2026-10-08; Swift sending implemented
2026-10-08 through the existing adapter.

Decision:

One new message in the existing JSON envelope, no new C functions. Swift sends it
once, after acting on a picked result:

```text
request   {"op":"record_selection","query":"c",
           "shown":["/System/Applications/Chess.app","/System/Applications/Calendar.app"],
           "selected":"/System/Applications/Calendar.app","outcome":"opened"}
response  {"kind":"recorded"}
error     {"kind":"error","reason":"bad_request","message":"selected must be one of shown"}
```

- `shown` is every result path in display order when the user acted. `selected`
  must be one of them. At most 100 paths; all absolute.
- `outcome` is `opened` (app opened or brought forward) or `failed`. Only
  `opened` counts as a launch in the usage store. Both are logged.
- The engine stamps time on receipt. Clients do not send timestamps.
- Effects, in order, under one lock: append one line to `events.jsonl`; if
  opened, update the usage store; save `usage.json` atomically. The log goes
  first because it is the raw record the store could be rebuilt from. `recorded`
  means both are on disk.

Files, in `~/Library/Application Support/Milky/`:

- `events.jsonl` — append-only, one versioned record per selection
  (`v`, `event`, `at_ms`, `query`, `shown`, `selected`, `outcome`). Raw history
  for offline evaluation and later learned ranking.
- `usage.json` — decayed launch scores per app path, derived from selections.
  Written atomically (temp file, `sync_all`, rename).

Why:

The launcher needs usage to rank, and later learning needs to know what was
passed over, not only what was picked. Logging the shown list with each
selection captures that without a per-keystroke impression stream. The Swift
side originally sketched four events (`impression`, `selection`,
`action_attempt`, `action_success`). In this launcher a selection *is* the
action, so one event with an outcome carries the same information with less
coordination.

Measured cost per selection on this machine: durable append p50 4.0 ms / p95
7.1 ms; atomic save p50 4.9 ms / p95 6.5 ms; about 9 ms / 14 ms together. It
runs once, after Enter, never on the keystroke path. Because the Swift adapter
serializes engine calls, a search typed right after a failed launch can wait up
to ~14 ms, under one frame. Writes therefore stay synchronous; no background
writer.

Consequences:

- A corrupt `usage.json` is renamed to `usage.json.corrupt-<unix seconds>` and
  the engine starts with empty history rather than refusing to start.
- Queries and app paths are stored locally and never leave the machine,
  consistent with the fully-local decision. Roughly 2 KB per selection; about
  36 MB a year at 50 selections a day. Rotation can wait.
- Not recorded in v1: dismissals without a selection (an abandonment signal
  worth adding later as another `op`), and per-keystroke impressions.
- Usage does not affect ranking yet; that is the next ranking step. When it
  does, searches will read the same lock, and a search may wait behind a
  selection write; revisit the lock type then.

---

## 2026-10-08 — Usage breaks ties within a match kind, and only that

Status: implemented.

Decision:

Sort results by match kind (strongest first), then decayed usage score
(highest first, `f64::total_cmp`), then name length, then name. Usage reorders
results that match equally well; it never lifts a result past a stronger match.

Why:

Letting usage outrank match strength requires a weight — how many launches one
match level is worth — and with no recorded selections any value would be
invented. A wrong weight makes results jump unpredictably. Tie-breaking alone is
a strict improvement: it only reorders results that were previously ordered by
length and alphabet. Example: `c` now puts a used Calendar above an unused Chess.

Consequences:

`co` still ranks Console (prefix) above a daily-used Visual Studio Code (word
prefix). Revisit once `events.jsonl` holds a few weeks of real selections:
replay them offline and compare this ranker against weighted blends by where
the user's pick lands. Pinned by the test `usage_never_beats_a_stronger_kind`.

---

## 2026-10-08 — App display names from LaunchServices; matching on precomputed folded keys

Status: implemented.

Decision:

- Display names: at engine start, Rust asks macOS LaunchServices for each app's
  localized name (`CFURLCopyResourcePropertyForKey`, `kCFURLLocalizedNameKey`,
  via the `core-foundation` crate plus one hand-declared function), strips the
  `.app` it keeps, and falls back to the bundle folder name.
- Matching: each app carries a `NameKey` computed once. Names and queries are
  folded the same way: NFKD, combining marks and invisible formatting characters
  removed, lowercased (`unicode-normalization`). Words split on whitespace and at
  camelCase boundaries. Whole-word initials and camelCase initials both count as
  acronyms.

Why:

5 of 127 apps showed the wrong name (`FindMy` for Find My, `zoom.us` for Zoom,
and so on); the fix matches Spotlight exactly for all 127. Supplying names from
Swift was the alternative. Rejected because it would put a data source in the UI
layer and make the engine's results depend on which client asked.

Costs, measured: display names take about 23 ms at engine start (0.2 ms per
app), off the UI thread, and again on reindex. Per-query matching got faster,
from 60-90 us to 5-13 us, because names are no longer re-derived on every
keystroke.

Consequences:

- Not full Unicode case folding: `ß` stays `ß`. Folding is deliberately lenient
  for some scripts: Japanese voiced marks are dropped, so `が` matches `か`.
- camelCase words can add mild extra word-prefix matches (`app` matches
  WhatsApp), which rank below true prefix matches.
- Dashes are dropped when folding (added 2026-10-08), so `wifi`, `wi-fi`, and
  Apple's `Wi‑Fi` (U+2011) are the same text.
- `libmilky_ffi.a` now depends on the CoreFoundation framework (and, since
  settings discovery, IOKit); every consumer must link them. The `CMilkyFFI`
  module map declares both, so Swift targets get them automatically.

---

## 2026-10-08 — Results carry a kind, an action, and a destination ID (contract v3)

Status: accepted by the user. Rust side implemented 2026-10-08
(`milky_abi_version()` = 3); Swift adapter not yet on v3.

Decision:

- Identity names the destination, not the text:
  `app:/Applications/Safari.app`, `settings:<pane bundle id>`,
  `settings:<pane bundle id>#<section anchor>`.
- One result per destination. A destination answers to several names (Wi-Fi's
  `Advanced` section is reached by "Advanced", "Wi‑Fi MAC Address", ...); the
  result shows whichever name matched best. A section item titled exactly like
  its pane is covered by the pane result.
- Wire shape of a result, replacing `path` and `name`:

```text
{"id":"settings:com.apple.Network-Settings.extension#802.1X",
 "kind":"setting",
 "title":"802.1X certificate",
 "subtitle":"Network",
 "action":{"type":"open_url","url":"x-apple.systempreferences:com.apple.Network-Settings.extension?802.1X"},
 "match_kind":"prefix"}
apps: "kind":"app", "action":{"type":"launch","path":"/Applications/..."}
```

- Subtitles: a section shows its pane name; a pane shows "System Settings"; an
  app shows its folder ("Python 3.12" — which disambiguates the three IDLEs).
- `record_selection` `shown` and `selected` carry result IDs. Event log records
  move to version 2; version-1 lines (paths) remain readable.
- Usage history is keyed by ID. Existing path keys migrate on load to
  `app:<path>`; nothing is discarded.
- Apps and settings rank together by the existing rules. Settings match pane
  names and section titles; keywords come later.
- Breaking change, so the version reported by `milky_abi_version()` moves from
  2 to 3; it covers the message format as well as the C signatures.

Why:

Measured on this Mac: 711 settings items lead to 561 destinations; 62
destinations are reached by several items (Menu Bar `?tutorial` by 21). Usage
should attach to where the user went, not which of its titles they typed, and
the Swift adapter rejects duplicate IDs within a response. Additive fields were
considered and rejected: settings have no path, so an old adapter would fail on
them anyway, and a clean version bump is caught by the existing startup check.

Consequences:

- Between the Rust change landing and the Swift adapter adopting v3, a rebuilt
  launcher refuses to search (version mismatch). Do not rebuild the daily
  launcher until both sides are on v3.

---

## 2026-10-08 — Keywords as separate evidence; ranking by tier, then usage, then destination prior

Status: implemented.

Decision:

- Multi-word queries: a title matches as `WordPrefix` when every query word
  starts some title word, in any order (`mac address`, `turn display`).
- Settings keywords (Apple's `.searchTerms` `index`) become a per-destination
  set of folded words. A keyword match is `Complete` (every query word is a
  whole keyword word) or `Partial` (every query word starts one). A pane takes
  the keywords of items titled like it.
- The first sort key is a `search::Tier`, weakest to strongest: Subsequence,
  Substring, KeywordPartial, Acronym, KeywordComplete, WordPrefix, Prefix,
  Exact. A title `WordPrefix` confirmed by a `Complete` keyword counts as
  `Prefix`.
- Order: tier, then usage, then prior (app > pane > section), then keyword
  match, then title length, title, ID. This refines "usage breaks ties within
  a match kind": usage still never outranks stronger evidence.
- Subsequence matches apply to apps only.
- No inverted index yet: a linear scan of ~4,700 keyword words keeps queries
  at ~75-90 us.

Why:

Measured against ~70 hand-written queries (settings plus app queries that
must not regress), across six designs in a scratch copy. Top-1 went from
51/73 to 60/73 and top-3 from 62/73 to 70/73. Findings that drove each rule:

- `dark mode`, `ssh`, `dnd`, `full disk access` matched nothing on titles.
- `camera` ranked Accessibility's keyword-less "Camera Options" (prefix) above
  the privacy setting whose keywords say "camera"; the confirmed-word rule is
  the only change that fixed it without regressing others.
- Settings titles already crowded apps on prefixes (`prev` → "Prevent
  automatic sleeping…" over Preview; `sys` → "System voice" over System
  Settings). The prior fixes this at cold start; usage overrides it.
- On sentence-long titles almost any short query is a subsequence: subsequence
  results in the top 5 fell from 67 to 5 across the set.
- An index pays for itself at file scale, not at 4,700 words.

Consequences:

- `match_kind` on the wire gains `"keyword"` (only keywords matched). Additive;
  Swift does not interpret it, so the ABI stays 3.
- Known misses: `location` ranks Family's "Location Sharing" first; `display`
  ranks Accessibility's "Display" above the Displays pane; plurals do not
  stem (`hot corners` vs keyword `hot corner`); `screen recording` has no
  matching keyword for the right entry.
- Pinned by `search::tests::real_queries_find_the_expected_destination` (19
  real queries on this Mac). The prior and tier ladder are hand-set guesses;
  revisit them with offline replay of `events.jsonl` once it has data.

---

## 2026-10-08 — File discovery: what Finder shows, minus generated and excluded folders

Status: implemented (walk only; not yet searchable).

Decision:

- Roots: the home folder and iCloud Drive
  (`~/Library/Mobile Documents/com~apple~CloudDocs`).
- Skip what Finder hides: names starting with `.`, and folders with the
  `UF_HIDDEN` flag (how `~/Library` is hidden, so it needs no special rule).
- Packages (`kCFURLIsPackageKey`: `.app`, `.pages`, `.photoslibrary`) are one
  file each and never entered. Symbolic links are neither followed nor listed.
- Folders that mark themselves as generated are skipped: `CACHEDIR.TAG`,
  `pyvenv.cfg`, `conda-meta`.
- Everything else generated is a rule in
  `~/Library/Application Support/Milky/exclusions.txt`: one per line, a bare
  name matches that folder name anywhere, a `~/` or `/` path matches one
  folder, `#` comments. Created with defaults (`node_modules`,
  `bower_components`, `Pods`, `__pycache__`, `~/go/pkg/mod`) when missing;
  never rewritten after that. Unparseable lines are reported, not dropped.
- Unreadable folders are reported in `Walk::unreadable`, not fatal.

Why:

Raw, the home folder here holds 3.2M entries (18 s to walk), almost all in
hidden tool folders; Finder's own rules bring it to ~23k in 0.05-0.3 s. Rules
must hold for any Mac, not be tuned to this one (user, 2026-10-08), so tool
output without a marker goes into a visible, editable list rather than code.
Names like `build`, `dist`, `vendor` are left out of the defaults because
people use them for their own folders. Reporting unreadable folders matters
because macOS privacy protection blocks the launcher per folder (Desktop,
Documents, Downloads, Photo Booth Library) and the user must be told.

Consequences:

Project source files are indexed (agreed). Exclusion rules match folders only,
not file patterns. `~/.Trash` contents are not indexed: reading it needs Full
Disk Access.

---

## 2026-10-08 — Well-known places, and contract v4 (folder and file results)

Status: contract implemented on Rust and Swift sides (2026-10-08); file results
began flowing through search on 2026-10-09.

Decision:

- `places.rs` lists 13 places Finder offers in its sidebar and Go menu: Home,
  Desktop, Documents, Downloads, Movies, Music, Pictures, Public, Library,
  Trash, iCloud Drive, Applications, Utilities. Only existing folders appear.
  Names come from LaunchServices (`launch_services.rs`, now the one home of
  the CoreFoundation calls). `~/Applications` and `/System/Applications` are
  left out: Finder's Applications already merges them.
- Aliases ("Home", "Bin", "Recycle Bin", "iCloud") are `Name`s with
  `alias: true`: they match at full strength but the result keeps its real
  title.
- Places are `Kind::Place`, prior just below apps (`App > Place > Pane >
  Section`): only `music` and `home` change, where the app wins.
- IDs are `folder:<path>`, the same ID a walked folder will get in step 3, so
  a place and the walked folder are one destination with one usage history.
- Contract v4 (ABI 4): result `kind` gains `folder` and `file`; `action` gains
  `{"type":"open","path":...}`. File results and `file:` IDs are emitted by
  file search (step 3, completed 2026-10-09). Selection events accept
  `folder:/...` and `file:/...` IDs.
- Subtitles: display paths, `~/Downloads` in the home folder and
  `iCloud Drive/...` inside iCloud Drive, as Finder names it.

Why:

Spotlight does not find basic folders like Downloads (user, 2026-10-08).
Bringing the contract forward lets places reach the launcher now instead of
after file search. Opening a place by path works for all of them, verified on
screen: `~/.Trash` opens Finder's real Trash view; only the window title shows
the path. Finder's AppleScript "open trash" would title it "Trash" but needs
an Automation permission prompt, which is not worth a title.

Consequences:

- Resolved 2026-10-08: the Swift adapter adopted v4 before rebuilding the
  launcher. The Rust engine began producing file results on 2026-10-09; no ABI
  bump was needed because v4 already defined file IDs, kinds, and open actions.
- Pinned by real-query tests: `downloads`, `down`, `documents`, `trash`,
  `recycle bin`, `icloud`, `applications`, `utilities`, `music`.

---

## 2026-10-09 — File matching and ranking: strong name matches, path support, a file cap

Status: implemented.

Decision:

- Names split into words at punctuation and between letters and digits, with
  each piece and the whole token also kept joined: `final-report_v2.pdf` ->
  `final`, `report`, `v`, `2`, `pdf`, `v2`, `finalreportv2pdf`.
- A file's name is matched without its extension (`resume` is exact for
  `resume.pdf`); folders keep their whole name.
- A file or folder matches only if every query word starts a word of its
  name, a folder above it (counted from iCloud Drive or the home folder), or
  its extension, and at least one word is in the name. The name's own words
  are scored with the usual ladder; anything below `WordPrefix` is no match
  (no substrings, initials, or scattered letters for files). If a folder or
  the extension was needed, the tier drops one step (`Exact` -> `Prefix` ->
  `WordPrefix` -> `PathSupported`, a new tier just below `WordPrefix`).
- Order: tier, usage, prior (apps > places > panes > sections > folders >
  files), keyword match, newest modification date, shorter title, title, ID
  (`search::rank`, total).
- Results: the best `limit` hits chosen with `select_nth_unstable_by` and a
  sort of just those, never a full sort. At most 40% of the list (at least
  1) are files or folders, the best ones; more only when nothing else fills
  the list.

Why:

Measured on this Mac. Loose matching on files is mostly noise: `notes`
matched 1,435 names loosely and 2 strongly. Of option D (path support counts
fully), B (always below name-only matches), and C (one step down), C was best
or tied in a known-file simulation (about 1,700 queries built from 500 real
files): folder+name queries found the file first 38% of the time vs 36%,
name+extension 37% vs 33-34%. Path support narrows the typical query from 21
matches to 5; without it those queries find nothing. Only 2% of files changed
in the last 30 days, so recency separates equally named files. The user asked
that results never fill with files, as Spotlight avoids; the cap enforces that
directly. The cap is a candidate for a user setting (TASKS, Later).

Consequences:

- `final report` is a `WordPrefix` (not `Prefix`) match for `final-report`,
  because folding drops the dash; `Final Report` is exact.
- Under 3 query characters (spaces aside; `search::MIN_FILE_QUERY_CHARS`) a
  file or folder must match exactly and then ranks as `WordPrefix`. At 1-2
  characters an exact file outranked every app (`a` -> a file "A"; `go` ->
  `~/go` above Google Chrome). A first version hid files entirely below 3
  characters; that hid `~/UW` for `uw`, where no app competes, so it was
  replaced the same day (see the next entry).

---

## 2026-10-09 — Queries split like names; naming a file is exact; the typed folder

Status: implemented. From the user's first day of daily use.

Decision:

- `normalize_query` treats punctuation and symbols as spaces (dashes are
  still folded away first, so `wi-fi` stays `wifi`). Queries and names go
  through it alike: `google.pdf` -> `google pdf`, `desktop/customized` ->
  `desktop customized`, `zoom.us` -> `zoom us`.
- A file or folder is an exact match when the query is its whole name with
  the extension, or a path ending in it, written as shown (`~/...`,
  `iCloud Drive/...`) or in full (`/Users/...`), compared component by
  component. The check allocates, so it runs only when the query ends with
  the file's whole name, tested by string comparison first.
- Short queries: see the amended consequence in the previous entry.
- When the best file matched with a folder's help, the deepest folder above
  it named by a query word the file's own name does not cover is added as
  the last result (replacing the last if the list is full; never with a
  limit below 2; never twice; only if that folder is indexed).

Why:

Pasted paths and the launcher's tab completion (which writes the result's
title, `Parth Kotwal Google.pdf`) found nothing: names were split at
punctuation, queries were not. The user wanted the folder a file was found
through offered too, to explore it; the folder the user typed (not the
file's parent) was chosen because it is the one they had in mind.

Consequences:

The first version of the path check ran on every matched file and made a
one-letter query take 15 ms and the FFI concurrency test 168 s; gating it
behind the whole-name comparison restored ~0.3 ms per query.

---

## 2026-10-12 — Head and tail: settings sections and files rank two steps lower

Status: implemented.

Decision:

- Hits are ordered by a "standing": the tier, lowered two steps
  (`search::TAIL_STEPS`) for the long tail, settings sections and walked
  files and folders, against the head: apps, places, and settings panes.
  Then usage, prior, and the rest as before. Within the head or the tail the
  order is unchanged.
- `.localized` folders are named, matched, and shown in paths by their Finder
  name (LaunchServices; fallback: drop the suffix): "Chrome Apps".

Why:

`chrome` put the folder "Chrome Apps.localized" (a prefix match) above Google
Chrome ("chrome" only starts its second word). Measured by typing a later
word of each app's name (67 queries): the app came first 14 times and a file
beat it 23 times (`word` -> Word.java, `settings` -> settings.rs). Lowering
files one step left 13; two steps left 0 but put long settings sections
("Resume auto scanning after selection") above an exactly named resume file.
Lowering sections with files gave 46/67 app-first, 0 file wins, `resume` still
file-first, `bluetooth` still pane-first, and every real-query test passing.

Consequences:

- No single ladder satisfies every case; this is a compromise. Usage can no
  longer lift a section or file above an app of equal raw strength
  (`usage_does_not_lift_the_tail_over_the_head`). Learned ranking from
  selections is the intended replacement (TASKS).

---

## 2026-10-08 — System Settings as a source: panes, sections, keywords, deep links

Status: implemented. Recorded 2026-10-13 from the working notes in TASKS,
where it lived until then.

Decision:

- Panes: every extension in `/System/Library/ExtensionKit/Extensions` (and
  System Settings' own PlugIns) with extension point
  `com.apple.Settings.extension.ui`; 51 on macOS 26. Some declare
  `presentsInSidebar: false` and are still searchable.
- Pane names, first that answers: the hardware-dependent sidebar key (Battery
  vs Energy, power source read via IOKit; Touch ID and Apple Intelligence
  assumed present), else `InfoPlist.loctable` (`en`), else `Info.plist`, else
  a cleaned identifier. Neither `Info.plist` nor LaunchServices alone is right
  for all panes.
- Sections: each pane's `en.lproj/<name>.searchTerms` plist, anchor ->
  `localizableStrings` entries with `title` and comma-separated `index`
  keywords (711 entries, 3,520 keywords). Entries sharing an anchor are one
  destination (606 settings results in all).
- Opening: `x-apple.systempreferences:<pane bundle id>` and
  `...:<pane bundle id>?<anchor>`. Verified on screen 2026-10-08 for
  `Privacy_Camera`, `nightShiftSection`, Wi-Fi `Advanced`; some sections open
  as a sheet, and a link sent while a sheet is closing can be ignored.

Why:

Spotlight finds "Bluetooth", "Night Shift", and "Camera" (privacy) in System
Settings; the user called this critical for daily use, ahead of files. The
search-terms files are what System Settings' own search uses, so their anchors
are exactly the deep-link anchors.

Consequences:

- Discovery costs ~40 ms at engine start (Info.plist scan 16 ms, loctables
  19 ms, search terms 6 ms); cacheable by OS version if it ever matters.
- English only. "AutoFill & Passwords" (under General) is in no search-terms
  file; General itself has no items; panes hidden by hardware (Mouse without a
  mouse, Classroom) are still listed. Accessibility is 342 of 711 entries,
  which is why sections rank as the long tail (DECISIONS 2026-10-12).
- Apple writes "Wi‑Fi" with U+2011, so dashes are folded away in matching.

---

## 2026-10-14 — A live file index: background start, snapshots, folder-level updates

Status: implemented 2026-10-14 (`engine.rs` `Snapshots`/`Index`, `indexer.rs`).

Decision:

- `Engine::new` loads apps, places, and settings and returns; the file walk
  runs on a background thread and its results are published when ready.
  Searches before then simply have no files. The engine reports whether files
  are ready (`files_ready`), an additive field Swift may show or ignore.
- The index is an immutable snapshot behind `Arc`; a search clones the
  pointer and runs without holding a lock; updates build a new snapshot and
  swap it in. Searches never see a half-updated index or wait on the indexer.
- FSEvents through the `notify` crate, watching the home folder, iCloud Drive,
  and the app folders. An event matters only if its folder is indexed (one
  set lookup); events are grouped for ~300 ms; each changed folder is re-read
  and diffed against the index (new folders walked, deleted folders removed
  with their contents); content edits only update the modified date. If
  FSEvents reports it lost track, rescan everything. The watcher starts before
  the initial walk so no change is missed.
- The same rules (hidden, packages, generated, exclusions) decide entries in
  full walks and per-folder updates.

Why:

Measured 2026-10-13, 90 s of normal use plus one Rust build: 37,004 path
events, peaking at 11,342/s; 86% in hidden folders, 10% in `~/Library`, and of
the remaining 1,566 all but one were in `target/`, which is never indexed. A
full rescan plus rebuild costs ~55 ms for ~5k files (20 ms walk, 33 ms build)
and grows with the file count, so rebuilding per event, or even per burst
during a build, is wasteful; re-reading one folder costs ~0.16 ms. `notify` is
the maintained FSEvents wrapper; hand-written bindings would mean C callbacks
for no gain, and only one module touches it, so it can be replaced later.

Consequences:

- Startup no longer grows with the number of files (~55 ms warm here).
- New dependency: `notify` (~25 small crates).
- The watcher can count per-folder activity cheaply, a future ranking feature
  (TASKS).

Implementation notes (2026-10-14), appended to the entry above:

- `Engine::index()` returns an `Arc<Index>`; `Engine::search(&index, ...)`
  borrows from it. The indexer thread holds a `FileIndex` (`BTreeMap` by
  path, so a folder's contents are one key range) and applies re-read folders
  with `FileIndex::apply_listing`. Messages: file events, `Rescan` (from
  `Engine::reindex`, now asynchronous), `Stop` (from `Drop`).
- The home folder is canonicalized at discovery: FSEvents reports real paths
  (`/private/var/...` for `/var/...`), and the index must use the same ones.
- Search replies carry `files_ready` (additive). Measured here: engine ready
  in ~0.11 s, files after ~0.18 s; a created or deleted file appears in under
  a second; a full Rust build cost the indexer 0.06 s of CPU, all filtered.
- `libmilky_ffi.a` now also needs the CoreServices framework (FSEvents).

---

## 2026-10-15 — Log interactions for learning: sessions, served features, context, focus

Status: phases 1 and 2 implemented on the Rust side 2026-10-16, with phase
3's export, pause, and delete; Swift sides in progress; retention pending.

Decision:

Record enough that a learned ranker (and later personalization) never has to
wait for new data to be collected. Four kinds of evidence:

1. Search sessions (Swift captures): invocation and how it ended (pick, Esc,
   click away, app switch); every query state with timestamps; which results
   were actually rendered for each; highlights, Tab completion, and the action
   taken (open, reveal, copy, inspect); time to pick; after a pick, whether
   the result stayed in front and for how long.
2. What the engine served (Rust): a `search_id` on every search reply,
   echoed by Swift in its events; for each search the top ~50 candidates (not
   only the visible ones) with their features at serving time (tier,
   standing, keyword match, usage score, prior, kind, file age, depth, file
   cap applied); the ranker version and index summary; latency. Features are
   logged because usage decays, files change, and rules change: they cannot
   be reconstructed later.
3. Context per invocation (Swift): frontmost app before invoking; time of day
   and weekday; running apps with a regular activation policy only (menu-bar
   and background agents such as Wispr Flow excluded); power and display.
4. Ambient behavior: app focus intervals all day (Swift, a resident observer
   on app activation), ended by screen sleep, lock, or a few minutes without
   input, so an app left in front overnight is not counted; files' macOS
   "last used" dates and per-folder activity (Rust).

Not logged: keystrokes outside Milky's field, clipboard contents, window
titles, file contents.

Storage: SQLite (the architecture's planned store for history), one database
in Milky's data folder, written off the search path. Full detail kept 90 days,
then compacted to summaries. `milky export` for the Python ML workspace;
pause and delete-history controls. Swift sends its events in batches through
a new additive `record_events` message, with `search_id` on search replies.

Phases: 1) search IDs, impressions with features, sessions with keystrokes,
timing, dismissals, and context, in SQLite; 2) post-pick dwell, focus
intervals, file last-used dates, folder activity; 3) retention, export,
privacy controls.

Why:

`events.jsonl` records only picks (16 after a week): no skips, no
abandonment, no timing, no context, and no record of why a result ranked where
it did. "Running" is a poor signal of use (agents and idle apps run all day),
so focus intervals with idle detection carry it instead; the model, not a
hand rule, decides how much each fact matters. Keystroke-level sessions plus
feature snapshots are ~0.5-1 MB per day of heavy use: too much for one
ever-growing text file, small for SQLite.

Consequences:

- `events.jsonl` is imported into the new store and then retired; `usage.json`
  stays until usage moves into the store too.
- The wire format is additive to contract v4 (ABI stays 4): `search_id` on
  search replies and a `record_events` request that replaces
  `record_selection` (specified in `swift/INTEGRATION.md`, 2026-10-15).

Implementation notes, phase 1 Rust (2026-10-16), for the entry above:

- `milky.sqlite3` (bundled SQLite via `rusqlite`, WAL, `synchronous=NORMAL`),
  schema versioned by `PRAGMA user_version` with append-only migrations.
  Tables: `destinations` (each result ID stored once), `sessions`,
  `session_running_apps`, `session_events`, `searches`, `impressions`
  (served rank, pool rank, and the features of `search::Features`), and
  `selections` (from `record_selection`, and the imported `events.jsonl`,
  which is then renamed `events.jsonl.imported`).
- Every search gets an ID (`<engine start ms>-<counter>`, hex) and is kept in
  memory (the last 512) with its shown results and its top 50 by rank, each
  with features. Only searches a session reports as `rendered` are stored:
  the decisive one (acted on, else last drawn) with its whole pool, others
  with what they showed. Cost measured: 0-0.1 ms per search.
- `record_events` derives usage from `action` events with outcome `opened`
  and action open, reveal, or copy path; inspect and menu do not count.
- `search::RANKER_VERSION` is stored with every search; bump it when ranking
  rules change.
- `MILKY_DATA_DIR` moves the engine's history elsewhere. Tests (Rust and
  Swift) must set it or use a temporary folder: until 2026-10-16 the Rust
  tests opened the real data folder (ISSUES).

Implementation notes, phases 2 and 3 (2026-10-16), for the entry above:

- Phase 2 was first planned for later; that was wrong for anything that only
  exists if recorded when it happens (dwell, focus, last opened, folder
  activity), so it was done at once. Phase 3 loses nothing by waiting, except
  retention, needed before data turns 90 days old.
- Last opened: the `com.apple.lastuseddate#PS` extended attribute (16 bytes,
  seconds and nanoseconds since 1970, little-endian) is what Spotlight's
  `kMDItemLastUsedDate` reads; `files::last_used` reads it with one
  `getxattr` per entry during the walk. Opening a file rewrites it, so the
  live index sees a change in that folder. 243 of 360 files in Downloads
  had it. Walk time unchanged within noise.
- Folder activity: the indexer counts changed paths per indexed folder per
  batch (`Plan::changes`), stores the raw counts (`folder_activity`), and
  keeps decayed scores (two-hour half-life, `activity.rs`). Served files and
  folders carry their folder's and the whole area's score as features.
- `record_focus` stores focus intervals (deduplicated by app and start);
  `dwell` is a session event sent in a later report for the same session.
- Schema version 2 adds these columns and tables; version 3 adds `settings`.
  The real database upgraded in place, picks intact.
- `milky --pause-history` / `--resume-history` (nothing about use is stored,
  usage scores included), `--export PATH` (`VACUUM INTO`: Python reads the
  copy with its own `sqlite3` module, so no other export format is needed),
  `--forget-history all|DAYS` (all also resets usage scores).

---

## 2026-10-16 — Document search: our own inverted index over extracted text

Status: agreed; being planned into beats (TASKS, Now).

Decision:

- Index the text inside documents (PDF, Word/RTF/ODT, Markdown, text,
  LaTeX) and code. Leave out data formats (`.json`, `.csv`, `.html`) by
  default. A per-file cap on text read (1 MB).
- Extract with Apple's engines from Rust: PDFKit (`objc2-pdf-kit`) and
  AppKit's document readers (`objc2-app-kit`); read plain text directly.
  OCR (Vision) for scanned PDFs and images is the last beat of this feature.
- Cache extracted text in SQLite (`content.sqlite3`, a rebuildable cache, not
  history), keyed by path, modified date, and size; re-extract only changed
  files; the live indexer drives it, in a low-priority background queue.
- The index is ours: an inverted index (word -> documents) built in memory
  from the cache, living inside the `Index` snapshot, split into words with
  the same rules as names, scored with BM25, prefix match on the last query
  word. SQLite FTS5 is used only in tests, as an oracle for the same queries.
  A compact memory-mapped on-disk format is a later stage.
- Ranking: a content-only match ranks below every name match; at most 3 per
  result list; every query word must occur; within content matches, BM25,
  recency, last opened. Each content result carries a `snippet` (additive
  wire field) the launcher shows under the title.

Why:

Measured 2026-10-16: 256 PDFs (1,624 pages) hold only ~2M characters, 56 are
scans; PDFKit extracts all in 28.5 s once. Documents and code are ~50 MB of
text; data files would triple it and are mostly datasets. FTS5 over the 50 MB:
91 MB index, ~1 min build, 0.2-2 ms queries, so a library is not needed for
speed at this scale, and our own index fits better: it lives in the immutable
snapshot (no SQL or locks per keystroke), shares the name tokenizer, exposes
per-term statistics as ranking features, and can later replace the linear
name scan. Vision OCR measured at 0.05-0.07 s per image. The user wants
justified from-scratch components; this is the core of a search engine.

Consequences:

- More code to get right; the FTS5 oracle tests guard it.
- Startup rebuilds the index from cached text in the background until the
  on-disk format exists.

Refinements agreed 2026-10-16, measured on the user's files after deleting
`Projects/bondo` and leaving out data files, for the entry above:

- Text to index: 16.6 MB in 1,249 files (documents and code) plus ~2 MB of
  PDF text; 321,000 distinct words; 2.76M occurrences; 670,000 (word, file)
  pairs (4.1 occurrences per pair).
- Positions are stored (phrases, proximity, snippet placement): an estimated
  ~6 MB compressed against ~2 MB for counts only; ~30-40 MB in RAM for the
  first in-memory version.
- The text cache holds only text that is expensive to extract again (PDF,
  Word, OCR; ~3 MB). Plain text and code are re-read from the files.
- The on-disk index format is not built by assumption: build stage 1,
  measure the startup rebuild, and build the format once that passes ~1 s.
  At this size a whole-file rewrite (temp file, then rename) costs a few ms
  (SSD writes ~2-4.7 GB/s), so segments and merging (LSM) are not needed.
- Beats: D1 extraction and cache, D2 document words, D5 ranking, snippets,
  wire, FTS5 oracle (Claude); D3 the inverted index, D4 queries with BM25 and
  phrases, D6 OCR (user).

D1 and D2 implemented 2026-10-16 (`extract.rs`, `textcache.rs`, `terms.rs`,
`milky --extract`), measured on the user's files:

- 1,554 indexable files: 1,472 with text (20.0M characters, 4.37M word
  occurrences), 82 without (scans and empty files, remembered as such).
- Cold, first run: 24.8 s (PDFs 17.3 s, Word 2.6 s, plain 2.0 s, word
  splitting 3.0 s). Warm, from the cache: 1.0 s, of which splitting 0.82 s.
- Text cache: 259 entries, 3.3 MB, as estimated.
- Word splitting was first 3.0 s warm: a duplicate check scanned all of a
  token's parts per joined form, quadratic on minified code (one token of
  thousands of pieces). Checking only the one case where a duplicate can occur
  made it linear. An ASCII fast path for folding helped little.
- PDFKit and AppKit run on a background thread inside an autorelease pool.
  `libmilky_ffi.a` now also links PDFKit, CoreGraphics, AppKit, Foundation,
  and `objc` (module map updated; 39 Swift tests pass).

