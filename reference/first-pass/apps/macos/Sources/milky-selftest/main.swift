// Assertions over the Swift -> C ABI -> Rust boundary.
//
// These would normally be an XCTest target, but XCTest and swift-testing ship
// with Xcode and this machine has only the Command Line Tools. Since the
// boundary is hand-written unsafe code, going uncovered was not an acceptable
// alternative. Run with `make selftest`; exits non-zero on any failure.

import Foundation
import MilkyKit

var failures: [String] = []
var checks = 0

// Top-level code in Swift 6 runs on the main actor, so anything touching these
// globals must be main-actor isolated too.
@MainActor
func check(_ name: String, _ body: () throws -> Void) {
    checks += 1
    do {
        try body()
        print("  ok    \(name)")
    } catch {
        failures.append("\(name): \(error)")
        print("  FAIL  \(name): \(error)")
    }
}

struct Failure: Error, CustomStringConvertible {
    let description: String
}

func expect(_ condition: Bool, _ message: String) throws {
    if !condition { throw Failure(description: message) }
}

func expectEqual<T: Equatable>(_ actual: T, _ expected: T, _ label: String) throws {
    if actual != expected {
        throw Failure(description: "\(label): expected \(expected), got \(actual)")
    }
}

print("milky selftest")

check("engine starts and reports health") {
    let health = try MilkyEngine().health()
    try expectEqual(health.apiVersion, milkyAPIVersion, "api version")
    try expect(!health.engineVersion.isEmpty, "engine version was empty")
}

check("search round-trips the query") {
    let results = try MilkyEngine().search("terminal")
    try expectEqual(results.query, "terminal", "query")
    try expect(!results.candidates.isEmpty, "expected candidates")
}

check("search respects limit") {
    let results = try MilkyEngine().search("terminal", limit: 1)
    try expectEqual(results.candidates.count, 1, "candidate count")
}

// Quotes and backslashes must survive JSON encoding, the C string conversion,
// and parsing on the Rust side.
check("query with JSON metacharacters survives") {
    let awkward = #"he said "hi" \ path"#
    try expectEqual(try MilkyEngine().search(awkward).query, awkward, "query")
}

check("non-ASCII query survives") {
    let query = "café 日本語 🌌"
    try expectEqual(try MilkyEngine().search(query).query, query, "query")
}

check("invalid configuration throws engineUnavailable") {
    do {
        _ = try MilkyEngine(configuration: #"{"default_limit":0}"#)
        throw Failure(description: "expected a thrown error")
    } catch MilkyError.engineUnavailable {
        // expected
    }
}

check("malformed raw request comes back as an error response") {
    let response = try MilkyEngine().requestJSON("definitely not json")
    try expect(response.contains(#""kind":"error""#), "got \(response)")
}

// Cheap check that create/free ownership is balanced: unbalanced frees show up
// here as a crash rather than as a slow leak in the real app.
check("repeated create and destroy") {
    for _ in 0..<200 {
        _ = try MilkyEngine().search("terminal", limit: 1)
    }
}

print("")
if failures.isEmpty {
    print("\(checks) checks passed")
} else {
    print("\(failures.count) of \(checks) checks FAILED")
    exit(1)
}
