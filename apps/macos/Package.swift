// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Milky",
    platforms: [.macOS(.v14)],
    targets: [
        // The raw C ABI.
        .systemLibrary(name: "CMilkyFFI", path: "Sources/CMilkyFFI"),

        // Prints the ABI version, to prove the boundary works.
        .executableTarget(
            name: "milky-probe",
            dependencies: ["CMilkyFFI"],
            linkerSettings: [.unsafeFlags(["-L../../rust/target/debug"])]
        ),
    ]
)