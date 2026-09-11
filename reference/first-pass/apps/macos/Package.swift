// swift-tools-version:6.0
import Foundation
import PackageDescription

// The Rust staticlib lands in rust/target/<profile>/. Derive that path from this
// file's own location so the package builds from any working directory, which
// plain relative linker flags would not survive.
let packageDirectory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let repositoryRoot = packageDirectory      // apps/macos
    .deletingLastPathComponent()           // apps
    .deletingLastPathComponent()           // repo root
let rustProfile = ProcessInfo.processInfo.environment["MILKY_RUST_PROFILE"] ?? "debug"
let rustLibraryDirectory = repositoryRoot
    .appendingPathComponent("rust/target/\(rustProfile)")
    .path

let package = Package(
    name: "Milky",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "MilkyKit", targets: ["MilkyKit"]),
        .executable(name: "milky-probe", targets: ["milky-probe"]),
        .executable(name: "milky-selftest", targets: ["milky-selftest"]),
    ],
    targets: [
        // The raw C ABI. Nothing outside MilkyKit should import this.
        .systemLibrary(name: "CMilkyFFI", path: "Sources/CMilkyFFI"),

        // The only place in Swift that touches unsafe pointers.
        .target(
            name: "MilkyKit",
            dependencies: ["CMilkyFFI"],
            linkerSettings: [.unsafeFlags(["-L\(rustLibraryDirectory)"])]
        ),

        // Round-trips a query through the boundary from the terminal.
        .executableTarget(name: "milky-probe", dependencies: ["MilkyKit"]),

        // Assertions over the FFI boundary. An executable rather than a
        // testTarget because XCTest and swift-testing ship with Xcode, which
        // this machine does not have (see .agents/ISSUES.md). Port it to a real
        // test target once Xcode is installed.
        .executableTarget(name: "milky-selftest", dependencies: ["MilkyKit"]),
    ]
)
