# Issues

Persistent log of bugs, failures, dead ends, and gotchas we have actually hit.

Purpose: so a future session does not repeat a past mistake, and so hard-won
context about *why something broke* lives outside chat history.

This is not a task list. Planned work belongs in `TASKS.md`. Durable design
choices belong in `DECISIONS.md`. An entry belongs here when we lost time to
something and the lesson is not obvious from reading the code.

Use this format:

```text
## YYYY-MM-DD — Short symptom-first title

Status: open | resolved | worked around | wontfix
Area: rust-core | ffi | swift-app | indexer | ml | build | tooling | macos

Symptom:
What we observed, including exact error text when useful.

Cause:
What was actually wrong.

Fix / workaround:
What we did.

Lesson:
What a future session should do differently. Skip if there is nothing durable.
```

Keep entries short. Prune ones that stop being useful, but do not delete an
entry just because it is resolved: resolved entries are the point.

---

## 2026-09-10 — No full Xcode on the dev machine, so no XCTest

Status: resolved
Area: tooling

Symptom:

`xcodebuild` failed with "requires Xcode, but active developer directory
'/Library/Developer/CommandLineTools' is a command line tools instance". In Swift
packages, `import XCTest` failed with "no such module 'XCTest'", and
swift-testing was missing too — only its macro plugin was present.

Cause:

Only the Command Line Tools were installed. Both test frameworks ship inside
Xcode, as does `xcodebuild`.

Fix / workaround:

Xcode installed; `xcode-select -p` now points at
`/Applications/Xcode.app/Contents/Developer`. XCTest and `swift test` are
available, so Swift-side assertions can live in a real `testTarget`.

Lesson:

Both test frameworks come from Xcode, not from the Swift toolchain. On a fresh
machine, check `xcode-select -p` before assuming `swift test` will work.

---

## 2026-09-10 — `nm` floods stderr with "Unknown attribute kind" on Rust archives

Status: worked around
Area: tooling

Symptom:

`nm target/debug/libmilky_ffi.a` prints many lines of

```text
Unknown attribute kind (102) (Producer: 'LLVM22.1.8-rust-1.98.1-stable'
Reader: 'LLVM APPLE_1_1700.6.3.2_0')
```

plus `no symbols` for many archive members. Easy to mistake for a build failure.

Cause:

Rust embeds LLVM bitcode in its object files, and Rust's LLVM is newer than the
one Apple's `nm` is built from, so the bitcode reader does not recognise newer
attribute kinds. It is a complaint about the embedded bitcode, not about the
Mach-O symbol table. `no symbols` is normal for members with no global symbols.

Fix / workaround:

Symbols are still reported correctly. Send stderr to `/dev/null`:

```bash
nm -gU target/debug/libmilky_ffi.a 2>/dev/null | grep abi_version
```

For clean output instead, `rustup component add llvm-tools` provides an
`llvm-nm` built from the same LLVM as the compiler.

Lesson:

Noise on stderr is not failure. Check the exit code and whether the answer
actually came back before debugging.

---

## 2026-09-11 — App display names are not in the bundle, and not in Info.plist

Status: resolved
Area: rust-core

Symptom:

Deriving an app's name from its bundle directory gave `FindMy` for what macOS
displays as `Find My`. Reading `Info.plist` instead does not fix it, and breaks
other apps:

```text
bundle directory            Info.plist    Foundation (correct)
FindMy.app                  FindMy        Find My
Visual Studio Code.app      Code          Visual Studio Code
WhatsApp.app                <U+200E>WhatsApp    WhatsApp
```

Cause:

The user-visible name comes from macOS's localization layer, not from a field in
the bundle. `FindMy.app/Contents/Resources/en.lproj` is empty — system apps ship
with resources stripped — so the string "Find My" is not in the bundle at all.
It is reachable through Spotlight metadata (`kMDItemDisplayName`) or Foundation
(`FileManager.displayName(atPath:)`), both of which were correct for all three.

Fix / workaround:

Still using the bundle directory name. Wrong for a small number of apps, but
dependency-free and correct for the large majority.

The real options, undecided: call CoreFoundation from Rust via the
`core-foundation` crate, or have the Swift layer supply display names. The second
puts a data source on the UI side of a boundary meant to keep the UI thin, so it
interacts with where the indexer eventually lives. Decide when app search quality
actually suffers.

Lesson:

Do not assume macOS metadata lives in the file you would expect. Check against
what Finder shows (`mdls -name kMDItemDisplayName <path>`) before trusting a
field. This was caught only because a test printed real names from the machine —
worth keeping tests that touch the real filesystem for exactly this reason.

Resolution, 2026-10-08:

`apps::display_name` asks LaunchServices from Rust through CoreFoundation
(`CFURLCopyResourcePropertyForKey` with `kCFURLLocalizedNameKey`) and strips the
`.app` the localized name keeps. Exactly matches Spotlight for all 127 apps on
this machine; 5 had been wrong (`FindMy`, `VoiceMemos`, `zoom.us`, `RemotePlay`,
`logioptionsplus`). About 0.2 ms per app, paid once at engine start.

---

## 2026-09-13 — Swift binaries keep a stale Rust archive after Rust rebuilds

Status: worked around (`apps/macos/scripts/build.sh`; see `.agents/swift/ISSUES.md`)
Area: build

Symptom:

After the ABI version moved to 2 and `libmilky_ffi.a` was rebuilt, `swift build
--product milky-probe` reported the build complete, yet `milky-probe` still
printed `milky ABI version: 1`. A C program linked against the same archive
returned 2.

Cause:

The archive is found through `-L` in `linkerSettings.unsafeFlags` plus
`link "milky_ffi"` in the module map. SwiftPM does not treat that external
archive as a build input, so it does not relink when only the Rust side changes.

Fix / workaround:

Force a relink by deleting the linked binary (or cleaning). Verified: after
removing `milky-probe` and rebuilding, it printed 2. A durable fix belongs to the
Swift build setup (Swift owner), such as a dev script that builds Rust and then
forces a relink, or making the archive a tracked input.

Lesson:

"Build complete" does not mean Swift sees the latest Rust. Check
`milky_abi_version()` at runtime; this check is what caught it.

---

## 2026-10-08 — Swift link failed after Rust gained a CoreFoundation dependency

Status: resolved
Area: build

Symptom:

After `milky-core` started calling CoreFoundation for app display names,
`apps/macos/scripts/build.sh test` failed linking `milky-probe`:
`_CFURLCopyResourcePropertyForKey`, `_kCFURLLocalizedNameKey`, and other CF
symbols not found.

Cause:

A Rust `staticlib` records the system libraries it needs but does not link them;
the final link, done by Swift, must. Nothing on the Swift side asked for
CoreFoundation, and `milky-probe` imports no Apple framework that would bring it
in.

Fix / workaround:

`link framework "CoreFoundation"` in `apps/macos/Sources/CMilkyFFI/module.modulemap`,
so every target importing the archive links it. 21 Swift tests pass.

Lesson:

When a Rust dependency touches a system framework, list what the archive needs
and make sure the module map covers it:

```bash
cargo rustc --manifest-path rust/Cargo.toml -p milky-ffi --lib --crate-type staticlib -- --print native-static-libs
```

Currently `-liconv -framework CoreFoundation -lSystem -lc -lm`. Only
CoreFoundation is not already provided by a default Swift link.

