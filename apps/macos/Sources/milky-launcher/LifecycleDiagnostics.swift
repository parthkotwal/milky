import AppKit

/// Explicit local debugging only. No queries, paths, app names, or key contents.
@MainActor
final class LifecycleDiagnostics {
    private let file: FileHandle?

    init() {
        let prefix = "--diagnostics="
        if let argument = CommandLine.arguments.first(where: { $0.hasPrefix(prefix) }) {
            let url = URL(fileURLWithPath: String(argument.dropFirst(prefix.count)))
            FileManager.default.createFile(atPath: url.path, contents: nil)
            file = try? FileHandle(forWritingTo: url)
        } else { file = nil }
    }

    func record(_ event: String, panel: NSPanel?, session: InvocationSessionSnapshot? = nil) {
        guard let file else { return }
        var entry: [String: Any] = [
            "event": event,
            "time": Date().timeIntervalSince1970,
            "visible": panel?.isVisible ?? false,
            "key": panel?.isKeyWindow ?? false,
            "ownsFocus": NSWorkspace.shared.frontmostApplication?.processIdentifier == ProcessInfo.processInfo.processIdentifier,
        ]
        if let session {
            entry["presented"] = session.presented
            entry["generation"] = session.generation
        }
        if let data = try? JSONSerialization.data(withJSONObject: entry, options: [.sortedKeys]) {
            try? file.write(contentsOf: data + Data([10]))
        }
    }

    func close() { try? file?.close() }
}

struct InvocationSessionSnapshot {
    let presented: Bool
    let generation: Int
}
