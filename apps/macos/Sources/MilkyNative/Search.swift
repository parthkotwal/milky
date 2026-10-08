import Foundation

public struct AppResult: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let url: URL

    public init(id: String, name: String, url: URL) {
        self.id = id
        self.name = name
        self.url = url
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
        app("Notes", "/System/Applications/Notes.app"),
        app("TextEdit", "/System/Applications/TextEdit.app"),
        app("System Settings", "/System/Applications/System Settings.app"),
        app("IDLE", "/Applications/Python 3.12/IDLE.app"),
        app("IDLE", "/Applications/Python 3.13/IDLE.app"),
        app("Milky Research — A Very Long Application Name for Layout Inspection", "/tmp/Milky Fixtures/Research/An intentionally long parent directory/Application Previews/Milky Research.app"),
        app("Unavailable Application", "/tmp/Milky Fixtures/Unavailable.app"),
    ]

    private static func app(_ name: String, _ path: String) -> AppResult {
        AppResult(id: path, name: name, url: URL(fileURLWithPath: path))
    }

    public func search(query: String) async throws -> [AppResult] {
        let key = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        try await Task.sleep(for: .milliseconds(key == "slow" ? 1200 : 60))
        if key == "error" { throw FixtureError.unavailable }
        if key == "all" || key == "slow" { return Self.results }
        return Self.results.filter { $0.name.localizedCaseInsensitiveContains(key) }
    }
}

public enum FixtureError: LocalizedError {
    case unavailable
    public var errorDescription: String? { "The fixture search failed. Try again or change your query." }
}
