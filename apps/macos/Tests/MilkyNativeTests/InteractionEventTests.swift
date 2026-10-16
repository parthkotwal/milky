import XCTest
@testable import MilkyNative

@MainActor final class InteractionEventTests: XCTestCase {
    func testRecordEventsEncodingMatchesAgreedWireShapeAndNullContext() throws {
        let context = InteractionContext(frontmostApp: nil, runningApps: ["app:/Applications/Terminal.app"],
                                         timezoneOffsetMin: -420, onBattery: nil, displays: 2)
        let session = InteractionSession(sessionID: "session-1", startedAtMs: 1_792_040_000_000,
            endedAtMs: 1_792_040_000_900, invocation: "hotkey", end: "action", context: context,
            events: [
                .query(t: 0, query: "s"),
                .rendered(t: 215, searchID: "s1f", visible: 8),
                .highlight(t: 700, searchID: "s1f", resultID: "app:/Applications/Safari.app", index: 1),
                .tabComplete(t: 750, searchID: "s1f", resultID: "app:/Applications/Safari.app"),
                .action(t: 900, action: "open", searchID: "s1f", resultID: "app:/Applications/Safari.app", index: 1, outcome: "opened"),
            ])
        let data = try JSONEncoder().encode(RecordEventsRequest(session: session))
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(root["op"] as? String, "record_events")
        let body = try XCTUnwrap(root["session"] as? [String: Any])
        XCTAssertEqual(body["session_id"] as? String, "session-1")
        XCTAssertEqual(body["started_at_ms"] as? Int64, session.startedAtMs)
        XCTAssertEqual(body["invocation"] as? String, "hotkey")
        XCTAssertEqual(body["end"] as? String, "action")
        let encodedContext = try XCTUnwrap(body["context"] as? [String: Any])
        XCTAssertTrue(encodedContext["frontmost_app"] is NSNull)
        XCTAssertTrue(encodedContext["on_battery"] is NSNull)
        XCTAssertEqual(encodedContext["running_apps"] as? [String], context.runningApps)
        let events = try XCTUnwrap(body["events"] as? [[String: Any]])
        XCTAssertEqual(events.map { $0["type"] as? String }, ["query", "rendered", "highlight", "tab_complete", "action"])
        XCTAssertEqual(events[1]["search_id"] as? String, "s1f")
        XCTAssertEqual(events[2]["result_id"] as? String, "app:/Applications/Safari.app")
        XCTAssertEqual(events[4]["outcome"] as? String, "opened")
        XCTAssertNil(events[0]["search_id"])
    }

    func testSessionRecorderAddsInitialQueryAndFinishesOnlyOnce() async throws {
        let recorder = InteractionSessionRecorder()
        recorder.begin(invocation: "other", context: InteractionContext(frontmostApp: "app:/a", runningApps: [], timezoneOffsetMin: 0, onBattery: false, displays: 1), initialQuery: "saved")
        recorder.query("saved")
        recorder.query("saved text")
        let session = recorder.finish(end: "escape")
        let batch = try XCTUnwrap(session)
        XCTAssertEqual(batch.events, [.query(t: 0, query: "saved"), .query(t: batch.events[1].time, query: "saved text")])
        XCTAssertEqual(batch.end, "escape")
        let secondFinish = recorder.finish(end: "other")
        XCTAssertNil(secondFinish)
    }

    func testDwellUsesTheOriginalSessionHeaderAndExplicitNullableApp() throws {
        let original = InteractionSession(sessionID: "session-2", startedAtMs: 1000, endedAtMs: 1500,
            invocation: "hotkey", end: "action",
            context: InteractionContext(frontmostApp: nil, runningApps: [], timezoneOffsetMin: 0, onBattery: nil, displays: 1),
            events: [.action(t: 500, action: "open", searchID: "search", resultID: "file:/tmp/a", index: 0, outcome: "opened")])
        let dwell = original.addingDwell(t: 2500, resultID: "file:/tmp/a", app: nil, frontMS: 1000, ended: "idle")
        XCTAssertEqual(dwell.sessionID, original.sessionID)
        XCTAssertEqual(dwell.startedAtMs, original.startedAtMs)
        XCTAssertEqual(dwell.endedAtMs, original.endedAtMs)
        XCTAssertEqual(dwell.invocation, original.invocation)
        XCTAssertEqual(dwell.end, original.end)
        XCTAssertEqual(dwell.context, original.context)
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(RecordEventsRequest(session: dwell))) as? [String: Any])
        let body = try XCTUnwrap(root["session"] as? [String: Any])
        let event = try XCTUnwrap((body["events"] as? [[String: Any]])?.first)
        XCTAssertEqual(event["type"] as? String, "dwell")
        XCTAssertEqual(event["t_ms"] as? Int, 2500)
        XCTAssertTrue(event["app"] is NSNull)
        XCTAssertEqual(event["ended"] as? String, "idle")
        XCTAssertEqual(event["front_ms"] as? Int, 1000)
        XCTAssertEqual(body["session_id"] as? String, "session-2")
    }

    func testRecordFocusWireShape() throws {
        let interval = InteractionFocusInterval(app: "app:/Applications/Ghostty.app", startedAtMs: 10,
                                                endedAtMs: 25, endedBy: "switch")
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(RecordFocusRequest(intervals: [interval]))) as? [String: Any])
        XCTAssertEqual(root["op"] as? String, "record_focus")
        let row = try XCTUnwrap((root["intervals"] as? [[String: Any]])?.first)
        XCTAssertEqual(row["app"] as? String, interval.app)
        XCTAssertEqual(row["started_at_ms"] as? Int, 10)
        XCTAssertEqual(row["ended_at_ms"] as? Int, 25)
        XCTAssertEqual(row["ended_by"] as? String, "switch")
    }
}

private extension InteractionEvent {
    var time: Int {
        switch self {
        case .query(let t, _), .rendered(let t, _, _), .highlight(let t, _, _, _), .tabComplete(let t, _, _), .action(let t, _, _, _, _, _), .dwell(let t, _, _, _, _): t
        }
    }
}
