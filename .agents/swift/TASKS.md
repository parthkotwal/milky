# Swift tasks

## Current milestone

Invoke Milky, type a query, navigate real app results, and open or activate the
selected application. This is the native foundation for the broader product.
Swift can progress with fixtures while the user and core agent build the FFI.

## Now

- [x] Establish the runnable native app target and document its launch command.
- [x] Implement the panel, search field, list, and selection state against an
      explicit fixture provider, following DESIGN.md.
- [ ] Implement invocation, dismissal, focus behavior, keyboard navigation, and
      native open/activate actions. Choose a configurable shortcut without
      modifying the user's system shortcuts. Implementation landed; physical
      shortcut/focus testing and the remaining QA below are still outstanding.
- [ ] Coordinate the minimal search boundary described in INTEGRATION.md with
      the core owner, then connect the real adapter when available.

Completion requires a built, running native app; repeated shortcut invocation;
responsive typing; real Rust results; duplicate-name disambiguation; correct
Enter/Escape behavior; and successful open/activation. Exercise no-result and
failure paths and inspect both appearances. A fixture-only UI is useful progress,
but does not complete the milestone. Record any unavailable validation honestly.

## Soon

- [ ] Integrate real impression/selection reporting and confirm local persistence
      with the core owner before sustained ranking-data collection.
- [ ] Refine icon loading, layout, accessibility, and focus based on actual use.
- [ ] Add result previews and secondary actions for the next real retrieval source.

## Later

- [ ] Present unified file/document/image results as core capabilities arrive.
- [ ] Extend preferences only when there are real user-facing settings to expose.

Keep this queue short. Put implementation findings in ISSUES.md and durable
choices in DECISIONS.md. Do not duplicate the shared engine backlog here.


## Run the fixture slice

From the repository root:

```sh
apps/macos/scripts/run-fixtures.sh
```

This builds the native product without Rust, wraps it in an ignored development
`.app` under `apps/macos/.build/`, and opens with `--fixtures`. Alternatively:

```sh
swift run --package-path apps/macos milky-launcher --fixtures
```

Use Control–Option–Space, or launch with
`--shortcut=command-shift-space`. The menu bar magnifying-glass item offers Show
and Quit; Command-Q quits while Milky is active. Quit the running copy before
rebuilding/relaunching with different flags. `--appearance=light` and
`--appearance=dark` override only this panel for QA. Omit them to follow macOS.

Fixture inputs: ordinary name substrings, `all` (entire corpus), `slow` (1.2 s
delayed corpus), `error` (search failure), `zzzz` (no matches), `IDLE` (duplicate
names), `Unavailable` (missing-app action failure). Enter opens actual installed
apps at the explicit fixture URLs; missing paths fail visibly. No usage is saved.

## QA evidence — 2026-09-11

- Debug and release native product builds succeeded. `swift test --package-path
  apps/macos` passed six XCTest cases: stale success/error responses, Enter
  gating, duplicate identity, same-query refresh, removed selection, query reset,
  dismissal invalidation, empty/search/action errors and recovery, and bounded
  native icon data. The existing probe may require the Rust archive for a fresh
  whole-package test build; the native product itself does not link Rust.
- Inspected the actual `.app` on macOS 26.5 in light and dark appearances.
  Verified typing without clicking the field, Command-A replacement, Up/Down
  selection, duplicate-path disambiguation, long names/paths, scroll-to-selection,
  missing icons, no matches, explicit search failure, and missing-app feedback.
- Enter on Calculator dismissed the launcher and Calculator was subsequently
  running. The running-app activation branch is implemented but not independently
  established by the UI check.
- Missing-app failure then typing a new query worked without a click. Opaque
  semantic background replaced material after contrast variation during QA.
- AX inspection exposed full result names/paths, selected traits, search field
  name, and Retry. VoiceOver speech/navigation, numerical contrast measurement,
  IME composition, physical hotkey repetition, exact focus-return behavior,
  multiple displays/Spaces/full-screen apps, and OS accessibility toggles remain
  unverified. There is no animation or transparency in the final surface.
- CUA could not reliably inspect the hidden app after Escape or injected global
  shortcuts. Do not treat those timeouts as proof of the shortcut acceptance
  criteria; see ISSUES.md.

The current milestone remains open until the agreed Rust adapter returns real
results and native invocation/action acceptance is verified end to end.
