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

## 2026-09-14 — Make invocation a session independent of panel visibility

Decision: Track launcher invocation with `InvocationSession`, separate from
`NSPanel.isVisible`. It retains one previous-app PID for a session and consumes
it before dismissal. Workspace activation, rather than resign-key, establishes
that the user has actually left Milky. The default hotkey coalesces repeat
key-down events until a matching key-up.

Why: AppKit can send visibility and key-window notifications during dismissal,
and accessibility clients can transiently resign the panel. The previous design
could dismiss from a weak signal and had no explicit protection against repeat
hotkey delivery.

Consequences: Escape returns focus exactly once if Milky owns focus. An action
or application switch never pulls focus back. The panel is repositioned when
display parameters change and always fits the pointer display’s visible frame.
Lifecycle diagnostics are opt-in through `--diagnostics=<path>` and record no
queries, names, paths, or keystroke contents.

## 2026-10-08 — Add a ⌘Space trial shortcut option

Decision: Keep Control–Option–Space as the default and add
`--shortcut=command-space` for users who intentionally disable Spotlight's
⌘Space binding. Preserve `--shortcut=command-shift-space`.

Why: The user wants to try Milky as their Spotlight replacement without making
the alternate binding a global default for other users.

Consequences: The OS shortcut must be released in Keyboard Shortcuts before
Milky can register ⌘Space. The active shortcut is reflected in the menu-bar
Show item; registration failures retain the existing visible fallback.

## 2026-10-08 — Adapt native results to contract v3

Decision: Keep the Swift presentation value destination-oriented: stable `id`,
`kind`, `title`, `subtitle`, and a typed primary action (`launch` or `openURL`).
Decode and validate the full wire action before releasing Rust memory. Native
execution dispatches by action, while the view uses core titles/subtitles and
native icons for apps or an SF Symbol for settings.

Why: Settings have no application path, and an app's path is an action target,
not its identity. Keeping both in a typed Swift value makes duplicate display
names safe and lets the same list and selection state cover different result
types without teaching the UI the wire JSON shape.

Alternatives: Retaining an app-only `url` property would make settings awkward
and invite invalid file URLs. Deriving display subtitles from paths would
duplicate presentation policy already owned by core.

Consequences: ABI 3 IDs are used unchanged for list identity and usage events.
App IDs must match their absolute launch path; setting actions need a URL scheme.
The opener launches or activates apps and opens settings links through
`NSWorkspace`. Fixtures cover both kinds but still do not record usage. The
shared contract is documented in `INTEGRATION.md` and the shared decision log.

## 2026-10-08 — Keep results steady between keystrokes

Decision: Keep the prior list and highlight visible while a new query is in
flight, disable acting on those stale results, then animate the updated rows
with a short fade. Dim the held list only after the delayed search indicator
appears. Respect Reduce Motion by disabling result animation.

Why: Clearing the list on every typed character makes the launcher look as if
it flashes or reloads, even when search returns quickly. Holding the rows gives
the eye a stable reference while subtle identity-based transitions reveal what
changed. The existing generation guard still discards out-of-order responses.

Alternatives: Clearing immediately is unambiguous but causes repeated blanking.
A full-list crossfade or animated panel resize would be more noticeable and
make the interface feel slower. Showing old rows as actionable risks launching
for the wrong query, so opening stays gated until completion.

Consequences: A new query chooses its first result after completion, even if
that ID was selected for the previous query. Same-query refresh preserves the
selection when possible. The offscreen snapshot harness can preload a previous
query to inspect the held-results state without taking user focus.

## 2026-10-08 — Resolve System Settings icons from pane bundles

Decision: Use each setting result's pane bundle ID to find its installed
ExtensionKit `.appex`, then request that bundle's icon through
`NSWorkspace.icon(forFile:)`. Reuse the existing bounded off-main icon
rasterization and cache. Keep the SF Symbol fallback for panes without a
usable bundle icon.

Why: Rust already supplies the pane bundle ID, and the macOS extension bundle
contains the artwork used for that pane. `NSWorkspace` returns the pane-specific
Wi-Fi and Network icons when given their `.appex` paths. This uses AppKit's
public file-icon API instead of hardcoding artwork in Milky.

Alternatives: A hardcoded SF Symbol gives every setting the same generic mark.
Adding icon paths to the result payload would expand the Rust ↔ Swift contract
for metadata that Swift can resolve locally.

Consequences: Swift builds a one-time ID-to-bundle map from the same two system
roots used by the Rust settings index. Lookup runs off the main thread and is
cached with app icons. Missing bundles or icons retain the SF Symbol fallback;
no Rust or ABI change is required. Apple API reference:
https://developer.apple.com/documentation/appkit/nsworkspace/icon%28forfile%3A%29

## 2026-10-08 — Suggest the ranked top result without changing typed text

Decision: When a non-exact query has results, show the first ranked destination
in a compact chip beside the query and its native icon at the trailing edge.
Tab replaces the query with the result title and reruns search; Return retains
its existing meaning and opens the selected result. Keep a stale suggestion
visible during an in-flight refresh for visual stability, but disable acceptance
until that search completes. Suppress the chip when query and title match
case-insensitively.

Why: Spotlight's completion affordance makes the likely destination easy to
recognize, and the side icon reinforces that identity. A result-based chip also
supports abbreviations such as `vsc`, where app-name suffix completion cannot
be inferred from the typed characters. Keeping typed text untouched avoids
overwriting the user's query as ranking changes.

Alternatives: Inline suffix completion would require a text-prefix relationship
and would distort acronym matches. Automatically opening the top result on Tab
would make completion too destructive and conflict with the existing Return
action.

Consequences: The suggestion uses the existing ordered search response and
native icon cache, so it requires no contract or Rust changes. The explicit
`vsc` fixture supports offscreen visual QA; fixture icon availability follows
the host's installed-app state.

## 2026-10-09 — Use balanced density with result type labels

Decision: The user chose a hybrid of the balanced panel's 54 pt spacing and
inline suggestion with the compact panel's App, Setting, Folder, and File
labels. It is the default in real and fixture modes. The earlier three
candidate modes remain fixture-only for comparison.

Why: The mixed fixture exposed that type labels clarify a single ranked list
without giving up the calmer row spacing. Grouping by type would change the
core's result order, so the labels provide context in place.

Consequences: Swift does not regroup or rerank results. The footer and empty
prompt now describe all local result kinds.

## 2026-10-09 — Inspect files and expose native secondary actions

Decision: Embed `QLPreviewView` for selected file URLs. Add an in-panel action
menu opened with Command-K, navigated by Up/Down and Return, and closed with
Escape. Command-P opens file inspection directly. The menu derives Open,
Inspect File, Reveal in Finder, and Copy Path or Link from the existing typed
result action and kind. Native AppKit APIs perform reveal and clipboard work.

Why: The current v4 results already provide validated local URLs. Quick Look
can preview supported file types without a new extraction/indexing contract,
and an explicit menu keeps secondary actions discoverable without crowding
every result row.

Consequences: No Rust ABI, ranking, or discovery changes. Primary Open keeps
the existing selection event. Reveal and Copy are not sent as `opened` usage
events because that outcome currently means a successful primary launch/open;
a future action-specific event requires core agreement. The inspector and menu
close on a changed query or selection so stale URLs cannot be acted on.
