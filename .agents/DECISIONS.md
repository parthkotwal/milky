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

