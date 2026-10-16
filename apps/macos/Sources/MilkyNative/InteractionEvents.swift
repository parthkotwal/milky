import Foundation

public struct InteractionContext: Encodable, Sendable, Equatable {
    public let frontmostApp: String?
    public let runningApps: [String]
    public let timezoneOffsetMin: Int
    public let onBattery: Bool?
    public let displays: Int
    public init(frontmostApp: String?, runningApps: [String], timezoneOffsetMin: Int, onBattery: Bool?, displays: Int) {
        self.frontmostApp = frontmostApp; self.runningApps = runningApps
        self.timezoneOffsetMin = timezoneOffsetMin; self.onBattery = onBattery; self.displays = displays
    }
    enum CodingKeys: String, CodingKey {
        case frontmostApp = "frontmost_app", runningApps = "running_apps"
        case timezoneOffsetMin = "timezone_offset_min", onBattery = "on_battery", displays
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        if let frontmostApp { try c.encode(frontmostApp, forKey: .frontmostApp) }
        else { try c.encodeNil(forKey: .frontmostApp) }
        try c.encode(runningApps, forKey: .runningApps)
        try c.encode(timezoneOffsetMin, forKey: .timezoneOffsetMin)
        if let onBattery { try c.encode(onBattery, forKey: .onBattery) }
        else { try c.encodeNil(forKey: .onBattery) }
        try c.encode(displays, forKey: .displays)
    }
}

public enum InteractionEvent: Encodable, Sendable, Equatable {
    case query(t: Int, query: String)
    case rendered(t: Int, searchID: String, visible: Int)
    case highlight(t: Int, searchID: String, resultID: String, index: Int)
    case tabComplete(t: Int, searchID: String, resultID: String)
    case dwell(t: Int, resultID: String, app: String?, frontMS: Int, ended: String)
    case action(t: Int, action: String, searchID: String, resultID: String, index: Int, outcome: String)

    enum CodingKeys: String, CodingKey { case t = "t_ms", type, query, searchID = "search_id", visible, resultID = "result_id", index, action, outcome, app, frontMS = "front_ms", ended }
    enum Kind: String, Encodable { case query, rendered, highlight, tabComplete = "tab_complete", action, dwell }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .query(t, query): try c.encode(t, forKey: .t); try c.encode(Kind.query, forKey: .type); try c.encode(query, forKey: .query)
        case let .rendered(t, searchID, visible): try c.encode(t, forKey: .t); try c.encode(Kind.rendered, forKey: .type); try c.encode(searchID, forKey: .searchID); try c.encode(visible, forKey: .visible)
        case let .highlight(t, searchID, resultID, index): try c.encode(t, forKey: .t); try c.encode(Kind.highlight, forKey: .type); try c.encode(searchID, forKey: .searchID); try c.encode(resultID, forKey: .resultID); try c.encode(index, forKey: .index)
        case let .tabComplete(t, searchID, resultID): try c.encode(t, forKey: .t); try c.encode(Kind.tabComplete, forKey: .type); try c.encode(searchID, forKey: .searchID); try c.encode(resultID, forKey: .resultID)
        case let .dwell(t, resultID, app, frontMS, ended):
            try c.encode(t, forKey: .t); try c.encode(Kind.dwell, forKey: .type); try c.encode(resultID, forKey: .resultID)
            if let app { try c.encode(app, forKey: .app) } else { try c.encodeNil(forKey: .app) }
            try c.encode(frontMS, forKey: .frontMS); try c.encode(ended, forKey: .ended)
        case let .action(t, action, searchID, resultID, index, outcome): try c.encode(t, forKey: .t); try c.encode(Kind.action, forKey: .type); try c.encode(action, forKey: .action); try c.encode(searchID, forKey: .searchID); try c.encode(resultID, forKey: .resultID); try c.encode(index, forKey: .index); try c.encode(outcome, forKey: .outcome)
        }
    }
}

public struct InteractionSession: Encodable, Sendable, Equatable {
    public let sessionID: String
    public let startedAtMs: Int64
    public let endedAtMs: Int64
    public let invocation: String
    public let end: String
    public let context: InteractionContext
    public let events: [InteractionEvent]
    public init(sessionID: String, startedAtMs: Int64, endedAtMs: Int64, invocation: String,
                end: String, context: InteractionContext, events: [InteractionEvent]) {
        self.sessionID = sessionID; self.startedAtMs = startedAtMs; self.endedAtMs = endedAtMs
        self.invocation = invocation; self.end = end; self.context = context; self.events = events
    }
    enum CodingKeys: String, CodingKey {
        case sessionID = "session_id", startedAtMs = "started_at_ms", endedAtMs = "ended_at_ms"
        case invocation, end, context, events
    }
}

public struct RecordEventsRequest: Encodable, Sendable {
    public let op = "record_events"
    public let session: InteractionSession
    public init(session: InteractionSession) { self.session = session }
}

public struct InteractionFocusInterval: Encodable, Sendable, Equatable {
    public let app: String
    public let startedAtMs: Int64
    public let endedAtMs: Int64
    public let endedBy: String
    public init(app: String, startedAtMs: Int64, endedAtMs: Int64, endedBy: String) {
        self.app = app; self.startedAtMs = startedAtMs; self.endedAtMs = endedAtMs; self.endedBy = endedBy
    }
    enum CodingKeys: String, CodingKey { case app, startedAtMs = "started_at_ms", endedAtMs = "ended_at_ms", endedBy = "ended_by" }
}

public struct RecordFocusRequest: Encodable, Sendable {
    public let op = "record_focus"
    public let intervals: [InteractionFocusInterval]
    public init(intervals: [InteractionFocusInterval]) { self.intervals = intervals }
}

public extension InteractionSession {
    func addingDwell(t: Int, resultID: String, app: String?, frontMS: Int, ended: String) -> InteractionSession {
        InteractionSession(sessionID: sessionID, startedAtMs: startedAtMs, endedAtMs: endedAtMs,
            invocation: invocation, end: end, context: context,
            events: [.dwell(t: t, resultID: resultID, app: app, frontMS: frontMS, ended: ended)])
    }
}

/// Main-actor session accumulator. It stores only the agreed event fields and
/// produces one immutable batch at dismissal.
@MainActor public final class InteractionSessionRecorder {
    private var sessionID: UUID?
    private var startDate: Date?
    private var startUptime: ContinuousClock.Instant?
    private var invocation = "other"
    private var context: InteractionContext?
    private var events: [InteractionEvent] = []
    private var lastQuery: String?

    public init() {}

    public func begin(invocation: String, context: InteractionContext, initialQuery: String) {
        sessionID = UUID(); startDate = Date(); startUptime = ContinuousClock.now
        self.invocation = invocation; self.context = context; events = []; lastQuery = nil
        query(initialQuery)
    }

    private func elapsed() -> Int {
        guard let startUptime else { return 0 }
        let duration = startUptime.duration(to: .now).components
        return max(0, Int(duration.seconds * 1000 + duration.attoseconds / 1_000_000_000_000_000))
    }
    private func append(_ make: (Int) -> InteractionEvent) { guard sessionID != nil, events.count < 2_000 else { return }; events.append(make(elapsed())) }
    public func query(_ value: String) {
        guard value != lastQuery else { return }; lastQuery = value
        append { .query(t: $0, query: String(value.prefix(500))) }
    }
    public func rendered(searchID: String, visible: Int) { append { .rendered(t: $0, searchID: searchID, visible: visible) } }
    public func highlight(searchID: String, resultID: String, index: Int) { append { .highlight(t: $0, searchID: searchID, resultID: resultID, index: index) } }
    public func tabComplete(searchID: String, resultID: String) { append { .tabComplete(t: $0, searchID: searchID, resultID: resultID) } }
    public func action(_ action: String, searchID: String, resultID: String, index: Int, succeeded: Bool) {
        append { .action(t: $0, action: action, searchID: searchID, resultID: resultID, index: index, outcome: succeeded ? "opened" : "failed") }
    }

    public func finish(end: String) -> InteractionSession? {
        guard let id = sessionID, let startDate, let context else { return nil }
        let start = Int64(startDate.timeIntervalSince1970 * 1000)
        let session = InteractionSession(sessionID: id.uuidString, startedAtMs: start,
            endedAtMs: start + Int64(elapsed()), invocation: invocation, end: end,
            context: context, events: events)
        sessionID = nil; self.startDate = nil; startUptime = nil; self.context = nil
        events = []; lastQuery = nil
        return session
    }
}
