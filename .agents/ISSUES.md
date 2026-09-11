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
