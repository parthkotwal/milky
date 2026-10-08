# Swift issues

Record native UI, lifecycle, build, accessibility, and integration failures
actually encountered. Keep tasks in TASKS.md and decisions in DECISIONS.md.
Retain resolved entries when their lesson remains useful.

Entry format:

```text
## YYYY-MM-DD — Symptom
Status: open | resolved | worked around
Area: panel | input | lifecycle | actions | accessibility | build | integration
Symptom:
Cause:
Fix / workaround:
Lesson:
Validation:
```

See the shared `../ISSUES.md` for Xcode, archive-tooling, and app-display-name findings.

## 2026-09-11 — Search field did not initially receive typing
Status: resolved
Area: input
Symptom: The initial panel appeared but typing had no effect.
Cause: NSHostingView's field attachment and application/key-window activation
complete after initial window construction.
Fix / workaround: Lay out the host, focus immediately when possible, and defer
focus on the main queue after showing and after windowDidBecomeKey.
Lesson: Verify real keyboard input after AppKit activation, not only the view tree.
Validation: Typed queries into fresh launches without clicking the field.

## 2026-09-11 — Native icon export was disproportionately large
Status: resolved
Area: lifecycle
Symptom: Icons appeared late and exporting one app icon yielded 73,949,448 bytes.
Cause: NSImage TIFF export includes all native icon representations.
Fix / workaround: Rasterize to a 64 × 64 bitmap on a utility task, cache PNG data,
and instantiate the display image on the main actor.
Lesson: Bound icon dimensions before serializing/caching native representations.
Validation: Real icons visible; regression test asserts 64 × 64 and <100 KB.

## 2026-09-11 — Action failure lost keyboard focus and obscured selection
Status: resolved
Area: actions
Symptom: The missing-app failure was visible, but typing did not resume and a
bottom result could disappear behind the new feedback region.
Cause: Disabling the field during an action resigns its field editor. Feedback
also shrinks the result viewport without changing the selected ID.
Fix / workaround: Restore focus when re-enabling a key-window field and scroll
the selected result into view after error layout changes.
Validation: After failed launch, typing a new query worked without clicking.

## 2026-09-11 — Command-A did not select search text
Status: resolved
Area: input
Symptom: Typing after Command-A appended to the query.
Cause: A bare NSApplication executable lacks the Edit menu's responder-chain
key equivalents.
Fix / workaround: Install native Edit actions (undo/redo, cut/copy/paste/select
all) and Quit in the application menu.
Lesson: A working field editor alone does not provide a complete app menu.
Validation: See TASKS.md QA notes for the final keyboard check.

## 2026-09-11 — Hidden-panel shortcut QA could not be established through automation
Status: resolved
Area: lifecycle
Symptom: CUA timed out inspecting the hidden app after injected Control–Option–Space.
Cause: Not established; injected app-targeted keys may not exercise Carbon's
global hotkey route. Process sampling showed an idle main event loop, not a hang.
Fix / workaround: Add opt-in local lifecycle diagnostics and test physically.
Validation: On 2026-09-14 the user completed two successful shortcut → type →
Escape cycles from another app. Diagnostics showed accepted focus return. CUA
still cannot establish this interaction by itself.

## 2026-09-14 — Accessibility clicks impersonate an external app activation
Status: worked around
Area: lifecycle
Symptom: A CUA accessibility click on a result caused the panel to hide.
Cause: The automation harness changes the foreground application while invoking
the accessibility action. The workspace observer correctly interpreted that as
the user switching apps.
Fix / workaround: Do not use CUA accessibility clicks as physical-mouse QA.
Also remove dismissal-on-resign-key: that notification alone is too weak and can
occur transiently. Only a confirmed workspace activation dismisses the panel.
Lesson: Distinguish AppKit key-window callbacks from a real foreground-app change.
Validation: Lifecycle unit tests pass; physical shortcut/focus cycles pass.

## 2026-09-13 — Stale Rust archive handling in native builds
Status: worked around
Area: build
Symptom: Core owner observed SwiftPM retaining ABI 1 after rebuilding ABI 2.
Cause: External archive changes do not invalidate SwiftPM's linked products.
Fix / workaround: Canonical build/run scripts fingerprint the archive and clean
Swift products on content changes. Absolute search paths work from any cwd.
Lesson: Use `scripts/build.sh` or `scripts/run.sh` after Rust edits; raw SwiftPM
commands bypass this invalidation. Keep runtime ABI validation as a second check.
Validation: First fingerprint-miss build cleaned and rebuilt all products; probe
reports ABI 2, real Swift bridge tests pass, unchanged fingerprint reuses products.
Shared root cause remains recorded in `../ISSUES.md` (2026-09-13).
