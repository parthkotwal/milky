// swift-tools-version:6.0
import PackageDescription
import Foundation

let rustLibraryDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().appendingPathComponent("../../rust/target/debug").standardized.path
let rustLinkerSettings: [LinkerSetting] = [.unsafeFlags(["-L", rustLibraryDirectory])]

let package = Package(
    name: "Milky",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "MilkyNative"),
        .target(name: "MilkyRust", dependencies: ["MilkyNative", "CMilkyFFI"], linkerSettings: rustLinkerSettings),
        .executableTarget(name: "milky-launcher", dependencies: ["MilkyNative", "MilkyRust"]),
        .executableTarget(name: "milky-snapshot", dependencies: ["MilkyNative"]),
        .testTarget(name: "MilkyRustTests", dependencies: ["MilkyRust"]),
        .testTarget(name: "MilkyNativeTests", dependencies: ["MilkyNative"]),
        // The raw C ABI.
        .systemLibrary(name: "CMilkyFFI", path: "Sources/CMilkyFFI"),

        // Prints the ABI version, to prove the boundary works.
        .executableTarget(
            name: "milky-probe",
            dependencies: ["CMilkyFFI"],
            linkerSettings: rustLinkerSettings
        ),
    ]
)
