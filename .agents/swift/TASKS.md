# Swift tasks

## Current milestone

Invoke Milky, type a query, navigate real app, settings, folder, and file
results, and execute the selected destination's primary action. This is the
native foundation for the broader product. Real Rust search is connected.
Explicit fixtures remain available for UI QA. Contract v4 results render native
icons and execute their declared launch, URL, or open-path actions. Files can be
inspected through Quick Look; secondary actions are available from the keyboard.

## Now

- [x] Establish the runnable native app target and document its launch command.
- [x] Implement the panel, search field, list, and selection state against an
      explicit fixture provider, following DESIGN.md.
- [x] Implement invocation, dismissal, focus behavior, keyboard navigation, and
      native open/activate actions. Control–Option–Space is configurable and
      does not modify system shortcuts. Core acceptance passed on 2026-09-14.
- [x] Connect the agreed ABI 2 search boundary through the real Swift adapter.
- [x] Adopt ABI 3 result IDs, kinds, titles, subtitles, actions, and ID-based
      selection events; support opening System Settings deep links.
- [x] Build Rust before Swift and invalidate stale static linkage in the dev scripts.
- [x] Add a Spotlight-inspired top-result chip: preserve typed text, complete
      the query with Tab, and show the result's icon at the trailing edge.
- [x] Adopt ABI 4 folder/file result kinds and the typed `open` path action;
      open destinations natively and use their Finder icons.

Status: native app-search foundation complete. The remaining checks below are
quality follow-ups; they do not block the next product capability.

## Next

1. Run focused dogfooding QA with a physical mouse, VoiceOver enabled, a second
   display, Spaces, and a full-screen app. Fix only failures actually observed.
2. Run interaction QA for mixed app/setting results and confirm a settings
   deep link opens the intended pane. Keep the initial row/Enter action fast.
3. Dogfood the Tab suggestion with real Rust results, including an acronym
   query such as `vsc`; verify completion leaves opening to Return.
4. In the rebuilt launcher, search for a known project file and verify Return
   opens it in its default app; confirm folder results still open in Finder.

## Soon

- [x] Integrate `record_selection` through the agreed Rust contract after each
      real open attempt; fixtures remain excluded from persistence.
- [x] Choose the hybrid mixed-result panel; add Quick Look file inspection and
      a keyboard-accessible native action menu from existing URLs.
- [ ] Dogfood physical mouse, VoiceOver, multi-display, Spaces, and full-screen
      behavior; record only concrete failures in ISSUES.md.
- [ ] Add previews and secondary actions when a core result type supplies them.

## Later

- [ ] Present document contents, OCR results, and images as core capabilities arrive.
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
`--shortcut=command-shift-space` selects the alternate and
`--shortcut=command-space` selects ⌘Space (disable Spotlight's matching
shortcut first). The menu bar item offers
Show/Quit; Command-Q quits an active panel. `--appearance=light` or `dark` applies
only to this panel. Omitting appearance flags follows macOS.

For explicit development fixtures:

```sh
apps/macos/scripts/run-fixtures.sh
```

This wraps `run.sh --fixtures` and now also builds the linked Rust archive.
Inputs: title/subtitle substrings, `all`, `slow`, `error`, `zzzz`, `vsc`, `IDLE`, `Unavailable`.
Real mode does not treat those words as fixture commands. Enter performs the
selected fixture action or opens the real app/settings destination in Rust
mode. Selection events are saved only in real Rust mode; fixture mode never
writes usage history.

To inspect a launcher state without activating a window or taking keyboard
focus, render an offscreen PNG:

```sh
apps/macos/scripts/snapshot.sh --query all --appearance dark --output /tmp/milky-all.png
apps/macos/scripts/snapshot.sh --query zzzz --appearance light --output /tmp/milky-empty.png
apps/macos/scripts/snapshot.sh --query slow --during-search --output /tmp/milky-loading.png
apps/macos/scripts/snapshot.sh --previous-query all --query slow --during-search --output /tmp/milky-refreshing.png
```

The snapshot executable uses explicit fixtures, never opens results or records
usage, and verifies the foreground app is unchanged. Generated images default
to the ignored `apps/macos/.build/snapshots/` directory when `--output` is not
provided. `--previous-query` first resolves a fixture query so an in-flight
snapshot can show old rows held in place while a new query is loading.

## QA evidence — 2026-09-14 — Invocation and accessibility pass

- The user physically ran two Control–Option–Space → type → Escape cycles from
  another app. Both opened ready for typing and returned focus to the prior app.
  Local opt-in lifecycle diagnostics recorded both `hotkey`, `dismiss-return`,
  and `return-accepted` transitions. This replaces the earlier unverified
  shortcut/focus status.
- Invocation state now consumes its return target before ordering the panel out,
  making reentrant dismissal inert. Repeated hotkey key-down notifications are
  coalesced until key-up. A real activation of another app dismisses without
  trying to steal focus back; merely resigning key status does not dismiss.
- Placement is clamped to the pointer display’s visible frame and recomputed on
  display-parameter changes. Tests cover primary, portrait, negative-origin,
  and smaller-than-panel display frames.
- Escape is handled by the native field editor and by the panel, so it works if
  focus is no longer in the field. Up/Down, Return, standard text editing,
  Command-A, click selection, and double-click opening share selection state.
  CUA accessibility clicks change the foreground app and are therefore not a
  trustworthy mouse-drag/click simulation; no physical-mouse claim is made.
- AX inspection exposes selected traits, result paths, field usage help, and
  Retry. VoiceOver receives selected-result, error, and no-match announcements.
  The secondary text contrast is tested at at least 4.5:1 on normal and selected
  surfaces in light and dark appearance. The opaque surface has no motion or
  transparency dependency.
- `apps/macos/scripts/build.sh test` passes 18 tests: 6 lifecycle/placement/
  contrast cases, 7 state/icon cases, and 5 real Rust-bridge cases. Real
  Rust searches, duplicate app paths, no matches, and Calculator opening were
  verified in the native panel earlier in this milestone.

Still unverified: physical mouse drag behavior, VoiceOver speech/navigation in
an enabled VoiceOver session, multiple active displays/Spaces/full-screen apps,
and toggling macOS accessibility settings. These are QA follow-ups, not blockers
to the implemented invocation behavior.

## QA evidence — 2026-10-08 — Result location and state polish

- Result subtitles now show the final two parent-folder components, separated
  by a chevron. The two fixture IDLE results visibly read `Applications ›
  Python 3.12` and `Applications › Python 3.13`; VoiceOver and hover still
  expose the complete absolute path.
- The empty, populated, no-match, and fixture search-error states were inspected
  in the running dark-mode panel. Live Rust search for `visual studio` showed
  the correct Visual Studio Code result with the same compact location label.
- Queries inside the 180 ms grace period no longer show the empty-state icon;
  slower searches receive an explicit searching presentation after the delay.
- `apps/macos/scripts/build.sh test` passes 21 tests, including location-label
  formatting and the existing light/dark contrast checks.

## QA evidence — 2026-10-08 — Offscreen snapshot harness

- `apps/macos/scripts/snapshot.sh --query all --appearance dark` rendered a
  2× PNG without changing the frontmost process. The snapshot uses fixture
  search and a non-activating AppKit policy; it does not show a window or
  require a real app launch.
- Available visual states are controlled by `--query`: omit for the empty
  prompt, use `all` for a populated list, `zzzz` for no matches, and `error`
  for the fixture error state. `--query slow --during-search` captures the
  delayed loading state. `--appearance light|dark` and `--output PATH` select
  appearance and destination.

## QA evidence — 2026-10-08 — Ranked top-result suggestion

- An explicit `vsc` fixture returns Visual Studio Code as the top result. The
  dark offscreen snapshot shows `vsc` unchanged, a `Visual Studio Code` Tab
  chip, the native app icon at the trailing edge, and the selected result row.
  Snapshot verification confirmed the foreground app did not change.
- State tests verify a pending/stale suggestion cannot be accepted, Tab
  completion replaces the query without opening, and an exact title match
  hides the chip. The complete Swift suite passes 24 tests.

## QA evidence — 2026-10-08 — Contract v4 places

- `apps/macos/scripts/build.sh test` validates ABI 4 and passes 27 tests.
  Real Rust searches return Downloads, Applications, and Utilities as folder
  IDs with open-path actions; decoder tests also cover future file results and
  reject mismatched kinds, IDs, and relative paths.
- A dark offscreen `downloads` snapshot shows the system's native Downloads
  folder icon and `~/Downloads` subtitle without changing the foreground app.
- The app and settings paths still pass the same bridge suite. Opening a place
  in Finder through the live launcher remains a focused manual check.

## QA evidence — 2026-10-09 — File results through contract v4

- The rebuilt Rust archive now returns walked file and folder results using the
  already-agreed ABI 4 variants. `apps/macos/scripts/build.sh test` passes 28
  tests, including a real query returning this project's
  `LauncherStateTests.swift` file with its matching `file:` ID and `open` path.
- Native file and folder icons resolve from their paths, and the existing
  opener sends the result's declared path to `NSWorkspace.open`. The live
  launcher was rebuilt and restarted with the user's `--shortcut=command-space`
  setting preserved; visually checking and opening a file in its panel remains
  a keyboard QA step.

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

The native search, primary open, and first selection-event wiring are complete.
Next work is focused dogfooding of file inspection, native secondary actions,
and launcher behavior across displays and accessibility settings.

## QA evidence — 2026-10-09 — Hybrid panel and file actions

- The chosen hybrid combines 54 pt rows and the inline Tab suggestion with
  result-type labels. Dark and light snapshots of `mixed` results were inspected.
  Fixture-only `balanced`, `compact`, and `guided` variants remain runnable.
- File inspection and the action menu render through the offscreen snapshot
  tool. In the running fixture panel, typing `mixed`, selecting `Search.swift`
  with Down, Command-K, Down, Return opened its Quick Look preview. Escape
  returned to results; Command-P reopened inspection. The preview exposed the
  file's content to accessibility. The fixture app was quit after QA.
- `scripts/build.sh test` passes 30 tests, including new menu/inspection state
  transitions and the real Rust bridge suite. No Rust source or C ABI changed
  for this native work. In real mode, `Search.swift` came from Rust, Command-P
  displayed its contents, and Command-K → two Down presses → Return revealed
  the exact file selected in Finder. The QA Finder window and launcher were
  closed afterward. Physical clipboard behavior remains focused dogfooding QA.

## QA evidence — 2026-10-08 — Smoother query updates

- Changed queries now keep the current results and highlight steady until the
  replacement response arrives; actions remain disabled during that interval.
  A new query resets to its first result after the response, while same-query
  refresh keeps the selected destination if it still exists.
- Result insertions, removals, and reordering use a short fade. For searches
  lasting beyond the progress delay, old rows dim gently while the spinner is
  shown. Reduce Motion disables these effects.
- Snapshot harness accepts `--previous-query` to render the in-flight state
  without taking keyboard focus. An offscreen dark snapshot shows the previous
  results subdued beneath the delayed spinner. Automated state coverage verifies
  stale rows and their highlight remain visible, cannot launch under the new
  query, and selection resets to the first result when new results arrive.

## QA evidence — 2026-10-08 — Contract v3 adapter

- `apps/macos/scripts/build.sh test` passes 22 tests. The real linked Rust
  engine returns settings results for `wifi`; Swift decodes their destination
  IDs, titles, subtitles, and `x-apple.systempreferences` open actions. Fixture
  decoding also covers both app and setting result rows.
- Offscreen snapshots of the mixed fixture list and settings-only results were
  rendered in dark and light appearance. The settings rows use their pane icon
  and subtitle; app rows retain native icons and folder subtitles.
  Snapshot generation verified the foreground app stayed unchanged.
- The native opener's System Settings handoff is implemented but has not been
  activated in a live panel during this pass. Keep this as a focused manual
  check; the user may run Milky without requiring keyboard or screen focus from
  the test harness.

## QA evidence — 2026-10-08 — System Settings pane icons

- Settings results carry a pane bundle ID, and each ID resolves to a matching
  `.appex` under the same two macOS roots the Rust index scans. AppKit's
  `NSWorkspace.icon(forFile:)` reads the extension's associated icon directly;
  on this machine that returns the actual Wi-Fi and Network pane artwork.
- The fixture Wi-Fi rows now use real pane IDs. An offscreen snapshot shows the
  blue Wi-Fi pane icon in both the pane and its Advanced section, while app
  icons remain unchanged. Icon lookup and rasterization happen off the display
  path; the existing SF Symbol remains the fallback when no icon is available.
- No Rust changes or ABI changes were needed. All 23 Swift tests pass, including
  a bounded 64 × 64 settings-icon regression check.
