# Swift decisions

Durable native UI and design decisions only. Cross-language or product-wide
decisions belong in the shared `../DECISIONS.md`; link them here as needed.
Use dated entries with decision, why, alternatives, and consequences. Distinguish
an implementation default from a user-approved product commitment. Supersede
earlier entries explicitly when a decision changes.

## 2026-09-11 — Starting native design direction

Decision: Use a quiet, precise native launcher direction as the initial design
default, with deliberate system typography, a spacious query field, compact
results, and restrained selection styling. Details live in DESIGN.md.

Why: The shared brief prioritizes Spotlight-like simplicity and invocation speed
with richer actions. The user's frontend-design guidance calls for a coherent
point of view and careful execution; native interaction determines how to apply it.

Alternatives: Directly transplanting website typography, hero layouts, decorative
motion, or mobile breakpoints would not fit this interaction.

Consequences: Inspect and refine a real native panel. This is an initial direction,
not a finalized visual design approved by the user. Record substantial revisions.

## 2026-09-11 — Agents own Swift implementation without teaching detours

Decision: Per the user's explicit clarification, Swift agents own implementation
and technical design end to end. The user's involvement focuses on appearance
and interaction. Do not assign Swift exercises or routinely explain its internals.

Why: Learning Swift/UI architecture is not a goal for the user. The learning
priority remains Rust and the core systems.

Alternatives: Applying the shared learning-first workflow to every language would
spend the user's attention on material they do not want to study.

Consequences: This supersedes the earlier guideline to routinely explain native
activation, focus, concurrency, lifecycle, and ownership implementation choices.
Record those choices locally and explain user-visible consequences when relevant.
Swift-side FFI work can be implemented directly against the agreed contract;
coordinate with the core owner without taking over the user's Rust learning work.

## 2026-09-11 — Runnable fixture launcher with isolated presentation state

Decision: Add a `MilkyNative` library, `milky-launcher` executable, and XCTest
target. Require `--fixtures`; never silently substitute fixtures for Rust.
`SearchProvider` returns Swift-owned, Sendable presentation values. It does not
specify a C ABI. `LauncherState` owns request generations, stable-ID selection,
and action gating on the main actor. Native open/activate uses NSWorkspace.

Why: Native implementation can progress independently while the user owns Rust.
Cancellation alone cannot stop a late foreign response, so generation checks
also guard success, failure, dismissal, and action completion.

Alternatives: Direct view-to-FFI calls would couple UI development to an unsettled
boundary. Reimplementing discovery/ranking in Swift would violate ownership.

Consequences: Fixtures only filter fixed names in fixed order. No discovery,
ranking, event collection, persistence, or real engine adapter is claimed.
The existing probe/header and Rust source remain untouched.

## 2026-09-11 — Invocation and initial panel defaults

Decision: Use a 640 × 554 pt floating AppKit panel, bounded to the pointer's
screen visible frame, with SwiftUI content and an AppKit text field. Each
invocation begins empty. Default shortcut is Control–Option–Space; launch with
`--shortcut=command-shift-space` for the alternate. A menu bar item offers Show
and Quit, with explicit feedback if shortcut registration fails. Carbon hotkey
registration avoids requesting global keyboard monitoring permissions.

Why: These defaults preserve Spotlight's shortcut, maintain a steady viewport,
and keep native text editing/IME behavior. Escape restores the previous app
only if Milky still owns focus; a successful action never restores over its target.

Consequences: This is a development launch option, not a finished preferences UI.
Registration/handler cleanup occurs at termination. Repeated physical global
shortcut invocation, Space transitions, and focus return still need manual QA.

## 2026-09-11 — Opaque semantic surface and bounded icon data

Decision: Use the system window background as an opaque surface in both
appearances; do not animate layout. Native icons are rasterized to 64 × 64 PNG
on a utility task and cached, while rows initially show an SF Symbol fallback.

Why: Material contrast varied during actual focus/error inspection. An opaque
surface gives predictable contrast and already accommodates Reduce Transparency.
Native icon TIFF export included huge representations (~74 MB per app).

Alternatives: Keeping material with an opaque accessibility fallback still leaves
normal-mode contrast dependent on desktop content. Caching full native TIFFs
wastes memory for 32 pt icons.

Consequences: No wallpaper-dependent material or motion in this first panel.
Typography, selected stripe, and spacing carry the visual hierarchy. Cache size
is suitable for the fixed fixture corpus; revisit eviction for real large sources.

## 2026-09-13 — Connect the agreed JSON bridge and default to real search

Decision: Implement the accepted shared 2026-09-13 ABI 2 contract in a separate
`MilkyRust` target. The executable defaults to Rust; `--fixtures` remains explicit.
This supersedes the fixture-only launch requirement from 2026-09-11. The C header
and Rust exports are unchanged by the Swift integration.

Why: The core owner has delivered the contract. An actor-owned handle provides
simple off-main execution and safe shutdown without declaring pointers Sendable.
These short synchronous searches serialize in the adapter even though Rust
permits overlapping requests. Generation checks remain the stale-result guard.

Consequences: Engine preparation happens off-main after showing the panel. On
termination, AppKit waits for actor shutdown, which frees the handle only after
its active request finishes. Copied Data outlives foreign memory. Decoding does
not reorder, rename, or re-rank. Usage events remain a separate core agreement.

## 2026-09-13 — Make the development scripts responsible for fresh static linkage

Decision: `scripts/build.sh` builds Rust and compares the archive's SHA-256 to
its last successful Swift build. A difference cleans Swift products before
building/testing. `scripts/run.sh` invokes that build and packages the app;
`run-fixtures.sh` is now a mode wrapper. Use absolute library paths in SwiftPM.

Why: SwiftPM does not track a static library discovered through `-L` as an input.
A changed Rust archive must invalidate previously linked Swift products. A
content check avoids unnecessary clean builds when Cargo emits no changes.

Alternatives: A build plugin could track Cargo inputs, but is more infrastructure
than this package needs. Always cleaning would also work but slows UI iteration.

Consequences: Canonical scripts prevent stale linkage; direct SwiftPM remains
unsafe after Rust-only edits. Runtime ABI validation still rejects unsupported
versions before creating an engine. Fixture runs now also require the Rust
archive because both modes share one executable. Scripts refuse to overwrite a
running app; Quit before relaunching with changed code or flags.
