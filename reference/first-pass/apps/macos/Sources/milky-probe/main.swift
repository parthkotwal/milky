// Smallest possible proof that the whole stack works: Swift -> C ABI -> Rust
// engine -> JSON -> back. Run it with `make probe`.
//
// This is a terminal tool, not the launcher. The real app needs a window, a
// global hotkey, and an Info.plist, which needs full Xcode (see .agents/TASKS.md).

import Foundation
import MilkyKit

do {
    let engine = try MilkyEngine()

    let health = try engine.health()
    print("engine     \(health.engineVersion) (api \(health.apiVersion))")
    print("ready      \(health.ready)")
    if let note = health.note {
        print("note       \(note)")
    }

    guard health.apiVersion == milkyAPIVersion else {
        print("""
            warning: engine speaks api \(health.apiVersion) but MilkyKit \
            expects \(milkyAPIVersion)
            """)
        exit(1)
    }

    let query = CommandLine.arguments.dropFirst().joined(separator: " ")
    let results = try engine.search(query.isEmpty ? "terminal" : query)
    print("")
    print("query      \(results.query)")
    print("engine time \(results.tookMicros) us")
    for candidate in results.candidates {
        let subtitle = candidate.subtitle.map { " — \($0)" } ?? ""
        print(String(format: "  %.2f  [%@] %@%@",
                     candidate.score,
                     candidate.kind.rawValue,
                     candidate.title,
                     subtitle))
    }
} catch {
    FileHandle.standardError.write(Data("\(error)\n".utf8))
    exit(1)
}
