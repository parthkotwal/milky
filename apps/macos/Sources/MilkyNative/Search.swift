import Foundation

public enum ResultKind: String, Decodable, Sendable { case app, setting, folder, file }

public enum ResultAction: Equatable, Sendable {
    case launch(URL)
    case openURL(URL)
    case open(URL)
}

public struct AppResult: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: ResultKind
    public let title: String
    public let subtitle: String
    public let action: ResultAction

    public init(id: String, kind: ResultKind = .app, title: String, subtitle: String = "", action: ResultAction) {
        self.id = id
        self.kind = kind
        self.title = title
        self.subtitle = subtitle
        self.action = action
    }

    public var name: String { title }
    public var launchURL: URL? {
        if case .launch(let url) = action { return url }
        return nil
    }
    public var accessibilityDestination: String {
        switch action {
        case .launch(let url): url.path
        case .openURL: "Open in \(subtitle)"
        case .open(let url): kind == .folder ? "Open in Finder" : url.path
        }
    }
}

public protocol SearchProvider: Sendable {
    func search(query: String) async throws -> [AppResult]
}

public enum SelectionOutcome: String, Sendable {
    case opened
    case failed
}

public struct SelectionEvent: Sendable, Equatable {
    public let query: String
    public let shown: [String]
    public let selected: String
    public let outcome: SelectionOutcome

    public init(query: String, shown: [String], selected: String, outcome: SelectionOutcome) {
        self.query = query
        self.shown = shown
        self.selected = selected
        self.outcome = outcome
    }
}

public protocol SelectionEventRecording: Sendable {
    func recordSelection(_ event: SelectionEvent) async throws
}

/// Explicit development data. This is not app discovery or production ranking.
public struct FixtureSearchProvider: SearchProvider {
    public init() {}

    public static let results: [AppResult] = [
        app("Safari", "/Applications/Safari.app"),
        app("Calculator", "/System/Applications/Calculator.app"),
        app("Calendar", "/System/Applications/Calendar.app"),
        app("Visual Studio Code", "/Applications/Visual Studio Code.app"),
        app("Notes", "/System/Applications/Notes.app"),
        app("TextEdit", "/System/Applications/TextEdit.app"),
        setting("settings:com.apple.wifi-settings-extension", "Wi-Fi", "System Settings", "x-apple.systempreferences:com.apple.wifi-settings-extension"),
        setting("settings:com.apple.wifi-settings-extension#Advanced", "Advanced", "Wi-Fi", "x-apple.systempreferences:com.apple.wifi-settings-extension?Advanced"),
        app("IDLE", "/Applications/Python 3.12/IDLE.app"),
        app("IDLE", "/Applications/Python 3.13/IDLE.app"),
        folder("Downloads", FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Downloads")),
        folder("Utilities", URL(fileURLWithPath: "/Applications/Utilities", isDirectory: true)),
        folder("Trash", FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".Trash")),
        app("Milky Research — A Very Long Application Name for Layout Inspection", "/tmp/Milky Fixtures/Research/An intentionally long parent directory/Application Previews/Milky Research.app"),
        app("Unavailable Application", "/tmp/Milky Fixtures/Unavailable.app"),
    ]

    private static func app(_ name: String, _ path: String) -> AppResult {
        AppResult(id: "app:\(path)", title: name, subtitle: URL(fileURLWithPath: path).deletingLastPathComponent().lastPathComponent, action: .launch(URL(fileURLWithPath: path)))
    }

    private static func folder(_ name: String, _ url: URL) -> AppResult {
        AppResult(id: "folder:\(url.path)", kind: .folder, title: name,
                  subtitle: url.path.replacingOccurrences(of: FileManager.default.homeDirectoryForCurrentUser.path, with: "~"),
                  action: .open(url))
    }

    private static func setting(_ id: String, _ title: String, _ subtitle: String, _ url: String) -> AppResult {
        AppResult(id: id, kind: .setting, title: title, subtitle: subtitle, action: .openURL(URL(string: url)!))
    }

    public func search(query: String) async throws -> [AppResult] {
        let key = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        try await Task.sleep(for: .milliseconds(key == "slow" ? 1200 : 60))
        if key == "error" { throw FixtureError.unavailable }
        if key == "all" || key == "slow" { return Self.results }
        if key == "vsc" { return Self.results.filter { $0.title == "Visual Studio Code" } }
        return Self.results.filter { $0.title.localizedCaseInsensitiveContains(key) || $0.subtitle.localizedCaseInsensitiveContains(key) }
    }
}

public enum FixtureError: LocalizedError {
    case unavailable
    public var errorDescription: String? { "The fixture search failed. Try again or change your query." }
}
