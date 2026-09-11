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

Status: open
Area: tooling

Symptom:

`xcodebuild` fails with "requires Xcode, but active developer directory
'/Library/Developer/CommandLineTools' is a command line tools instance". In
Swift packages, `import XCTest` fails with "no such module 'XCTest'", and
swift-testing is missing too — only its macro plugin is present.

Cause:

Only the Command Line Tools are installed. Both test frameworks ship inside
Xcode, as does `xcodebuild`.

Fix / workaround:

Rust was installed via rustup (1.98.1) and is fine. On the Swift side, until
Xcode is installed, assertions have to live in a plain executable target rather
than a `testTarget`.

Lesson:

Do not assume `xcodebuild` or XCTest exist. Prefer SwiftPM-buildable targets for
anything that should run in a script or in CI.
