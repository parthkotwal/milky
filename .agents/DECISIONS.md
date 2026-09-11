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
