# Swift agent guidelines

## Purpose and collaboration

Build the native macOS experience for Milky. Follow `../AGENTS.md` alongside
these instructions. The user cares about how the app looks and interacts, but
does not want to learn Swift, native UI architecture, or how that implementation
is designed. Own the Swift implementation end to end, including architecture,
lifecycle, concurrency, tests, and debugging. Make routine technical decisions
autonomously; do not assign Swift exercises, pause for teaching, or require the
user to implement pieces themselves.

Keep user-facing discussion focused on visual design, interaction behavior,
demonstrable results, and meaningful product tradeoffs. Explain Swift internals
only when asked or when necessary to understand a concrete limitation or a
cross-language decision. Keep technical reasoning in the local records for
future agents. This does not reduce the quality or verification expected.

The learning-first approach still applies to Rust and core systems. Implement
the Swift side of an agreed FFI contract directly, while preserving the user's
learning work with the core owner on Rust exports and cross-language ownership.

Read DESIGN.md, INTEGRATION.md, TASKS.md, and relevant DECISIONS.md and ISSUES.md
before substantial work. Read the shared project memory too. Keep the ambitious
product direction while delivering narrow, usable interactions along the way.

## Ownership and coordination

- Swift agents own native UI, presentation state, keyboard interaction, app
  lifecycle, native action execution, and Swift tests under `apps/macos/`.
- Rust owns discovery, retrieval, ranking, indexing, and durable usage history.
  Do not reproduce those systems in Swift to make a demo look integrated.
- `Sources/CMilkyFFI/` is a shared boundary even though it sits under the Swift
  directory. Coordinate header/module changes with the core owner; declarations
  and Rust exports must agree. Follow INTEGRATION.md.
- Before implementation, state the files or component being owned. If multiple
  Swift agents are active, divide concrete files/components and designate one
  integration owner for shared app state, package changes, and the bridge.
  Do not start other agents unless the user has requested delegation.
- Inspect current changes before editing. Preserve other contributors' work.
  If ownership overlaps, coordinate the affected edit and continue independent work.
- Use isolated fixtures while the real boundary is unavailable. Label mock
  mode clearly in development and never silently fall back to it after a real error.
- Follow `reference/README.md`; do not mine the first pass for an implementation
  before attempting the corresponding work.

## Native implementation principles

- Use SwiftUI for view composition and AppKit where window behavior, focus,
  application activation, or native integration requires it. Keep the existing
  deployment target unless a justified change is coordinated.
- Separate panel lifecycle, presentation state, search access, and action
  execution enough to test behavior. Avoid a speculative framework or plugin system.
- Keep UI state on the main actor. Keep potentially blocking search and icon
  loading off the UI path; do not assume an FFI handle is thread-safe.
- Discard obsolete query responses. A result for an earlier query must not
  replace a newer one or launch because Enter raced with a refresh.
- Track selection by stable identity, not display name. Duplicate names are
  valid. Preserve selection for a refresh of the same query when still present;
  deliberately reset it for a new query.
- Handle observer registration, task cancellation, shortcut unregistration,
  engine lifetime, and result cleanup explicitly.
- Use native app APIs and file URLs for actions; do not build shell commands
  from result names or paths. Resolve icons through native facilities and cache
  them without delaying the result list.
- Do not print personal queries or full result paths into routine production
  logs. Keep any diagnostic content logging explicit and local.

## Records

Update only the files affected by meaningful work:

- DESIGN.md: current visual and interaction rules, including clearly marked
  defaults that can evolve through inspection.
- DECISIONS.md: dated, durable Swift/design choices with reasoning and consequences.
  Supersede old decisions rather than rewriting their history.
- TASKS.md: Swift work and concrete acceptance criteria.
- ISSUES.md: failures actually encountered, causes, workarounds, and lessons.
- INTEGRATION.md: verified boundary status and clearly separate pending proposals.

Swift-only updates satisfy the shared memory-update requirement for Swift-only
work. Do not duplicate each local task in the shared queue. For changes affecting
both sides, coordinate one shared decision/update with the core owner and link
to it here. A local proposal is not an agreed project contract.

## Verification and handoff

Build the targets touched. Test meaningful state transitions, stale responses,
selection behavior, action failures, and bridge ownership as those features land.
Inspect the running native app, not just SwiftUI previews: use light/dark
appearance, keyboard-only interaction, long and duplicate names, empty/error
states, repeated invocation, focus restoration, and multiple displays where available.
Check accessibility labels, contrast, Reduce Motion, and Reduce Transparency.

Report what runs, how to run it, what was actually verified, and what remains
mocked or unverified. A successful build alone does not establish visual quality
or correct global-shortcut/focus behavior. Keep QA evidence concise and local.
