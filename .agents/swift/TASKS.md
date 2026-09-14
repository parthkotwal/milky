# Swift tasks

## Current milestone

Invoke Milky, type a query, navigate real app results, and open or activate the
selected application. This is the native foundation for the broader product.
Real Rust app search is connected. Explicit fixtures remain available for UI QA.

## Now

- [x] Establish the runnable native app target and document its launch command.
- [x] Implement the panel, search field, list, and selection state against an
      explicit fixture provider, following DESIGN.md.
- [ ] Implement invocation, dismissal, focus behavior, keyboard navigation, and
      native open/activate actions. Choose a configurable shortcut without
      modifying the user's system shortcuts. Implementation landed; physical
      shortcut/focus testing and the remaining QA below are still outstanding.
- [x] Connect the agreed ABI 2 search boundary through the real Swift adapter.
- [x] Build Rust before Swift and invalidate stale static linkage in the dev scripts.

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


## Run and verify

From the repository root:

```sh
apps/macos/scripts/run.sh
apps/macos/scripts/build.sh test
```

`run.sh` builds Rust and Swift, assembles `.build/Milky.app`, and opens the real
launcher. Quit an already-running Milky before relaunching; the script refuses
to leave an old process running against a replaced binary. `build.sh` checks the
Rust archive fingerprint and cleans Swift build products when it changes. Use
these entry points after Rust edits; bare SwiftPM commands can retain stale Rust.

Control–Option–Space is the default shortcut;
`--shortcut=command-shift-space` selects the alternate. The menu bar item offers
Show/Quit; Command-Q quits an active panel. `--appearance=light` or `dark` applies
only to this panel. Omitting appearance flags follows macOS.

For explicit development fixtures:

```sh
apps/macos/scripts/run-fixtures.sh
```

This wraps `run.sh --fixtures` and now also builds the linked Rust archive.
Inputs: name substrings, `all`, `slow`, `error`, `zzzz`, `IDLE`, `Unavailable`.
Real mode does not treat those words as fixture commands. Enter opens real app
URLs in either mode. No usage is saved yet.

## QA evidence — 2026-09-13

- 12 XCTest cases pass through the canonical build script, including five bridge
  tests against the linked library/wire decoder and seven presentation/icon tests.
- Probe reports ABI 2. First script run invalidated old Swift products; subsequent
  unchanged archive reused them. Archive-content changes trigger the same clean
  path. Swift sources and Rust/header signatures remain independently owned.
- Actual app inspected in dark and light modes with Rust results. `visual studio`
  returns Visual Studio Code; `idle` shows three separate Python installations;
  `zzqqxxnomatch` shows no matches. Native app icons and paths are correct.
- Enter on Rust's Calculator result opened Calculator (absent from running-app
  inventory before the action, running afterward). Command-Q completed orderly
  termination; relaunch used the latest build. CUA may reopen a hidden app while
  inspecting it, so do not infer exact focus-return behavior from its snapshots.
- Physical repeated shortcut, focus return, multi-display/Spaces, VoiceOver, and
  the remaining accessibility checks below are still open. This integration
  does not mark the overall native milestone complete.

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

The current milestone remains open for native invocation/action acceptance;
the real Rust adapter requirement is now met.
