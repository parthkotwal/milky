import Foundation

/// One deterministic, real-path corpus for comparing visual concepts.
/// It never participates in normal Rust search or usage recording.
public struct DesignFixtureSearchProvider: SearchProvider {
    public init() {}

    private static let repository = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent("../../../..")
        .standardizedFileURL

    public static let results: [AppResult] = [
        folder(".agents/swift"),
        AppResult(id: "app:/Applications/Xcode.app", kind: .app, title: "Xcode",
                  subtitle: "Applications · Swift development",
                  action: .launch(URL(fileURLWithPath: "/Applications/Xcode.app"))),
        file("apps/macos/Sources/MilkyNative/LauncherView.swift"),
        file("apps/macos/Sources/MilkyNative/LauncherState.swift"),
        file("apps/macos/Sources/MilkyNative/Search.swift"),
        file("apps/macos/Sources/MilkyNative/FileInspectionView.swift"),
        file("apps/macos/Package.swift"),
        file("apps/macos/Sources/MilkyNative/NativeActions.swift"),
        file("apps/macos/Tests/MilkyNativeTests/PanelInteractionTests.swift"),
    ]

    private static func folder(_ relativePath: String) -> AppResult {
        let url = repository.appendingPathComponent(relativePath, isDirectory: true)
        return AppResult(id: "folder:\(url.path)", kind: .folder, title: "swift",
                         subtitle: "~/Projects/milky/.agents", action: .open(url))
    }

    private static func file(_ relativePath: String) -> AppResult {
        let url = repository.appendingPathComponent(relativePath)
        let parent = url.deletingLastPathComponent().path
            .replacingOccurrences(of: FileManager.default.homeDirectoryForCurrentUser.path, with: "~")
        return AppResult(id: "file:\(url.path)", kind: .file, title: url.lastPathComponent,
                         subtitle: parent, action: .open(url))
    }

    public func search(query: String) async throws -> [AppResult] {
        let key = query.trimmingCharacters(in: .whitespacesAndNewlines)
        try await Task.sleep(for: .milliseconds(45))
        guard !key.isEmpty else { return [] }
        if key.localizedCaseInsensitiveCompare("swift") == .orderedSame { return Self.results }
        return Self.results.filter {
            $0.title.localizedCaseInsensitiveContains(key) ||
            $0.subtitle.localizedCaseInsensitiveContains(key)
        }
    }
}
