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

