import XCTest
@testable import MilkyRust

@MainActor final class RustSearchProviderTests: XCTestCase {
    func testRejectsUnsupportedABIBeforeEngineCreation() throws {
        XCTAssertNoThrow(try RustSearchProvider.validateVersion(2))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(1))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(3))
    }

    func testRealEngineFindsInstalledAppsAndPreservesResultsAfterOtherRequests() async throws {
        let provider = RustSearchProvider(limit: 5)
        let results = try await provider.search(query: "term")
        XCTAssertTrue(results.contains { $0.name == "Terminal" && $0.id == "/System/Applications/Utilities/Terminal.app" })
        XCTAssertLessThanOrEqual(results.count, 5)
        let none = try await provider.search(query: "zzqqxxnomatch")
        XCTAssertTrue(none.isEmpty)
        // Earlier values must remain Swift-owned after later Rust allocation/free.
        XCTAssertTrue(results.contains { $0.name == "Terminal" })
        await provider.shutdown()
        XCTAssertTrue(results.contains { $0.url.lastPathComponent == "Terminal.app" })
        do {
            _ = try await provider.search(query: "term")
            XCTFail("Shutdown must prevent engine recreation")
        } catch BridgeError.closed {} catch { XCTFail("Unexpected shutdown error: \(error)") }
    }

    func testRepeatedConcurrentCallersAndJSONEscaping() async throws {
        let provider = RustSearchProvider(limit: 3)
        try await withThrowingTaskGroup(of: Void.self) { group in
            for index in 0..<100 {
                group.addTask {
                    let query = index.isMultiple(of: 2) ? "co" : "\"\\\n\0é🧪"
                    let values = try await provider.search(query: query)
                    XCTAssertLessThanOrEqual(values.count, 3)
                }
            }
            try await group.waitForAll()
        }
        await provider.shutdown()
    }

    func testDecodePreservesOrderIdentityUnicodeAndAllowsAdditionalFields() throws {
        let data = Data(#"{"kind":"search","query":"idle","results":[{"path":"/第二/IDLE.app","name":"IDLE","match_kind":"exact","future":42},{"path":"/first/IDLE.app","name":"IDLE","match_kind":"prefix"}]}"#.utf8)
        let values = try RustSearchProvider.decode(data, expectedQuery: "idle")
        XCTAssertEqual(values.map(\.id), ["/第二/IDLE.app", "/first/IDLE.app"])
        XCTAssertEqual(values.map(\.name), ["IDLE", "IDLE"])
    }

    func testErrorAndMalformedPayloadsNeverBecomeEmptyResults() throws {
        let error = Data(#"{"kind":"error","reason":"panic","message":"Engine failed"}"#.utf8)
        XCTAssertThrowsError(try RustSearchProvider.decode(error, expectedQuery: "a")) { error in
            guard case BridgeError.engine(let reason, _) = error else { return XCTFail("Expected engine error") }
            XCTAssertEqual(reason, "panic")
        }
        for text in [
            "not json",
            #"{"kind":"search","query":"old","results":[]}"#,
            #"{"kind":"search","query":"a","results":[{"path":"relative.app","name":"App"}]}"#,
            #"{"kind":"search","query":"a","results":[{"path":"/a","name":"A"},{"path":"/a","name":"A"}]}"#,
            #"{"kind":"unknown"}"#,
            #"{"kind":"error"}"#,
        ] {
            XCTAssertThrowsError(try RustSearchProvider.decode(Data(text.utf8), expectedQuery: "a"))
        }
    }
}
