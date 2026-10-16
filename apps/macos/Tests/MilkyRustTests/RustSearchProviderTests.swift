import XCTest
@testable import MilkyRust
import MilkyNative
import Darwin

@MainActor final class RustSearchProviderTests: XCTestCase {
    private static let isolatedDataDirectory = FileManager.default.temporaryDirectory
        .appendingPathComponent("MilkySwiftBridgeTests-\(UUID().uuidString)", isDirectory: true)
    private static var previousDataDirectory: String?

    override class func setUp() {
        super.setUp()
        MainActor.assumeIsolated {
            previousDataDirectory = ProcessInfo.processInfo.environment["MILKY_DATA_DIR"]
            try? FileManager.default.createDirectory(at: isolatedDataDirectory, withIntermediateDirectories: true)
            setenv("MILKY_DATA_DIR", isolatedDataDirectory.path, 1)
        }
    }

    override class func tearDown() {
        MainActor.assumeIsolated {
            if let previousDataDirectory { setenv("MILKY_DATA_DIR", previousDataDirectory, 1) }
            else { unsetenv("MILKY_DATA_DIR") }
            try? FileManager.default.removeItem(at: isolatedDataDirectory)
        }
        super.tearDown()
    }

    func testRejectsUnsupportedABIBeforeEngineCreation() throws {
        XCTAssertNoThrow(try RustSearchProvider.validateVersion(4))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(3))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(1))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(2))
    }

    func testRealEngineFindsInstalledAppsAndPreservesResultsAfterOtherRequests() async throws {
        let provider = RustSearchProvider(limit: 5)
        let response = try await provider.searchResponse(query: "term")
        let results = response.results
        XCTAssertFalse(try XCTUnwrap(response.searchID).isEmpty)
        XCTAssertTrue(results.contains { $0.name == "Terminal" && $0.id == "app:/System/Applications/Utilities/Terminal.app" })
        XCTAssertLessThanOrEqual(results.count, 5)
        let none = try await provider.search(query: "zzqqxxnomatch")
        XCTAssertTrue(none.isEmpty)
        // Earlier values must remain Swift-owned after later Rust allocation/free.
        XCTAssertTrue(results.contains { $0.name == "Terminal" })
        await provider.shutdown()
        XCTAssertTrue(results.contains { $0.launchURL?.lastPathComponent == "Terminal.app" })
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
        let data = Data(#"{"kind":"search","query":"idle","files_ready":false,"results":[{"id":"app:/第二/IDLE.app","kind":"app","title":"IDLE","subtitle":"第二","action":{"type":"launch","path":"/第二/IDLE.app"},"match_kind":"exact","future":42},{"id":"app:/first/IDLE.app","kind":"app","title":"IDLE","subtitle":"first","action":{"type":"launch","path":"/first/IDLE.app"},"match_kind":"prefix"}]}"#.utf8)
        let snapshot = try RustSearchProvider.decodeSearchSnapshot(data, expectedQuery: "idle")
        let values = snapshot.results
        XCTAssertFalse(snapshot.filesReady)
        XCTAssertEqual(values.map(\.id), ["app:/第二/IDLE.app", "app:/first/IDLE.app"])
        XCTAssertEqual(values.map(\.name), ["IDLE", "IDLE"])
        XCTAssertEqual(values.map(\.subtitle), ["第二", "first"])
    }

    func testSearchResponseRetainsOpaqueSearchID() throws {
        let data = Data(#"{"kind":"search","query":"x","results":[],"search_id":"opaque-1"}"#.utf8)
        let snapshot = try RustSearchProvider.decodeSearchSnapshot(data, expectedQuery: "x")
        XCTAssertEqual(snapshot.searchID, "opaque-1")
    }

    func testDecodeSettingsAndDiscriminatedActions() throws {
        let data = Data(#"{"kind":"search","query":"wifi","results":[{"id":"settings:com.apple.wifi#Advanced","kind":"setting","title":"Wi-Fi MAC Address","subtitle":"Wi-Fi","action":{"type":"open_url","url":"x-apple.systempreferences:com.apple.wifi?Advanced"},"match_kind":"prefix"}]}"#.utf8)
        let result = try XCTUnwrap(RustSearchProvider.decode(data, expectedQuery: "wifi").first)
        XCTAssertEqual(result.kind, .setting)
        XCTAssertEqual(result.action, .openURL(URL(string: "x-apple.systempreferences:com.apple.wifi?Advanced")!))
    }

    func testMissingFilesReadyMeansTheLegacySnapshotIsComplete() throws {
        let data = Data(#"{"kind":"search","query":"x","results":[]}"#.utf8)
        let snapshot = try RustSearchProvider.decodeSearchSnapshot(data, expectedQuery: "x")
        XCTAssertTrue(snapshot.filesReady)
    }

    func testRealEngineReturnsSettingsWithOpenURLActions() async throws {
        let provider = RustSearchProvider(limit: 20)
        let results = try await provider.search(query: "wifi")
        let setting = try XCTUnwrap(results.first { $0.kind == .setting })
        XCTAssertTrue(setting.id.hasPrefix("settings:"))
        XCTAssertFalse(setting.title.isEmpty)
        XCTAssertFalse(setting.subtitle.isEmpty)
        guard case .openURL(let url) = setting.action else {
            return XCTFail("Settings destinations must decode as open_url actions")
        }
        XCTAssertEqual(url.scheme, "x-apple.systempreferences")
        await provider.shutdown()
    }

    func testRealEngineReturnsWellKnownPlacesAsOpenableFolders() async throws {
        let provider = RustSearchProvider(limit: 20)
        let results = try await provider.search(query: "downloads")
        let downloads = try XCTUnwrap(results.first { $0.id == "folder:\(FileManager.default.homeDirectoryForCurrentUser.path)/Downloads" })
        XCTAssertEqual(results.first?.id, downloads.id, "The named place should rank first for its title")
        XCTAssertEqual(downloads.kind, .folder)
        XCTAssertEqual(downloads.title, "Downloads")
        guard case .open(let url) = downloads.action else {
            await provider.shutdown()
            return XCTFail("Places must decode as open path actions")
        }
        XCTAssertEqual(url.path, downloads.id.replacingOccurrences(of: "folder:", with: ""))

        let alias = try await provider.search(query: "recycle bin")
        XCTAssertEqual(alias.first?.id, "folder:\(FileManager.default.homeDirectoryForCurrentUser.path)/.Trash")
        XCTAssertEqual(alias.first?.title, "Trash", "Aliases find the destination without renaming it")
        await provider.shutdown()
    }

    func testDecodeFolderAndFutureFileKindsWithMatchingOpenPaths() throws {
        let data = Data(#"{"kind":"search","query":"notes","results":[{"id":"folder:/Users/a/Downloads","kind":"folder","title":"Downloads","subtitle":"~/Downloads","action":{"type":"open","path":"/Users/a/Downloads"}},{"id":"file:/Users/a/notes.txt","kind":"file","title":"notes.txt","subtitle":"~/notes.txt","action":{"type":"open","path":"/Users/a/notes.txt"}}]}"#.utf8)
        let values = try RustSearchProvider.decode(data, expectedQuery: "notes")
        XCTAssertEqual(values.map(\.kind), [.folder, .file])
        XCTAssertEqual(values.map(\.action), [
            .open(URL(fileURLWithPath: "/Users/a/Downloads")),
            .open(URL(fileURLWithPath: "/Users/a/notes.txt")),
        ])
        XCTAssertEqual(values.map(\.accessibilityDestination), ["Open in Finder", "/Users/a/notes.txt"])
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
            #"{"kind":"search","query":"a","results":[{"id":"app:relative.app","kind":"app","title":"App","subtitle":"","action":{"type":"launch","path":"relative.app"}}]}"#,
            #"{"kind":"search","query":"a","results":[{"id":"app:/a","kind":"app","title":"A","subtitle":"","action":{"type":"launch","path":"/a"}},{"id":"app:/a","kind":"app","title":"A","subtitle":"","action":{"type":"launch","path":"/a"}}]}"#,
            #"{"kind":"search","query":"a","results":[{"id":"folder:/else","kind":"folder","title":"Downloads","subtitle":"~/Downloads","action":{"type":"open","path":"/Users/a/Downloads"}}]}"#,
            #"{"kind":"search","query":"a","results":[{"id":"file:relative","kind":"file","title":"notes.txt","subtitle":"notes.txt","action":{"type":"open","path":"relative"}}]}"#,
            #"{"kind":"search","query":"a","results":[{"id":"setting:bad","kind":"setting","title":"Bad","subtitle":"","action":{"type":"open","path":"/tmp"}}]}"#,
            #"{"kind":"unknown"}"#,
            #"{"kind":"error"}"#,
        ] {
            XCTAssertThrowsError(try RustSearchProvider.decode(Data(text.utf8), expectedQuery: "a"))
        }
    }

    func testRealEngineReturnsApplicationsAndUtilitiesPlaces() async throws {
        let provider = RustSearchProvider(limit: 20)
        let utilities = try await provider.search(query: "utilities")
        XCTAssertTrue(utilities.contains { $0.id == "folder:/Applications/Utilities" && $0.kind == .folder })
        let applications = try await provider.search(query: "applications")
        XCTAssertTrue(applications.contains { $0.id == "folder:/Applications" && $0.kind == .folder })
        await provider.shutdown()
    }

    func testRealEngineReturnsProjectSourceFiles() async throws {
        let packageURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let sourceURL = packageURL.appendingPathComponent("Tests/MilkyNativeTests/LauncherStateTests.swift")
        XCTAssertTrue(FileManager.default.fileExists(atPath: sourceURL.path))

        let provider = RustSearchProvider(limit: 20)
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: .seconds(30))
        var snapshot = try await provider.searchSnapshot(query: "LauncherStateTests")
        while !snapshot.filesReady, clock.now < deadline {
            try await Task.sleep(for: .milliseconds(50))
            snapshot = try await provider.searchSnapshot(query: "LauncherStateTests")
        }
        XCTAssertTrue(snapshot.filesReady, "The background index should become ready during the test")
        let results = snapshot.results
        let source = try XCTUnwrap(results.first { $0.id == "file:\(sourceURL.path)" })
        XCTAssertEqual(source.kind, .file)
        XCTAssertEqual(source.title, "LauncherStateTests.swift")
        guard case .open(let url) = source.action else {
            await provider.shutdown()
            return XCTFail("Indexed files must decode as open path actions")
        }
        XCTAssertEqual(url.path, sourceURL.path)
        await provider.shutdown()
    }

    func testEventBatchAcknowledgementAndErrorsAreValidated() throws {
        XCTAssertNoThrow(try RustSearchProvider.validateRecorded(Data(#"{"kind":"recorded"}"#.utf8)))
        let rejected = BridgeError.engine(reason: "bad_request", message: "failed at /Users/person/private/path")
        XCTAssertEqual(rejected.safeDiagnostic, "bad_request")
        XCTAssertThrowsError(try RustSearchProvider.validateRecorded(Data(#"{"kind":"error","reason":"bad_request","message":"selected must be shown"}"#.utf8))) { error in
            guard case BridgeError.engine(let reason, _) = error else { return XCTFail("Expected engine error") }
            XCTAssertEqual(reason, "bad_request")
        }
        XCTAssertThrowsError(try RustSearchProvider.validateRecorded(Data(#"{"kind":"search","query":"x","results":[]}"#.utf8)))
        XCTAssertThrowsError(try RustSearchProvider.validateRecorded(Data(#"{"kind":"error"}"#.utf8)))
    }

    func testRealEngineAcceptsDocumentedSessionBatch() async throws {
        let provider = RustSearchProvider(limit: 5)
        let response = try await provider.searchResponse(query: "safari")
        let result = try XCTUnwrap(response.results.first)
        let searchID = try XCTUnwrap(response.searchID)
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let session = InteractionSession(sessionID: UUID().uuidString, startedAtMs: now,
            endedAtMs: now + 100, invocation: "other", end: "action",
            context: InteractionContext(frontmostApp: nil, runningApps: [], timezoneOffsetMin: 0, onBattery: nil, displays: 1),
            events: [.query(t: 0, query: "safari"), .rendered(t: 20, searchID: searchID, visible: 1),
                     .action(t: 100, action: "open", searchID: searchID, resultID: result.id, index: 0, outcome: "opened")])
        try await provider.recordEvents(session)
        let dwell = session.addingDwell(t: 250, resultID: result.id, app: "app:/Applications/Safari.app",
                                        frontMS: 150, ended: "switched")
        try await provider.recordEvents(dwell)
        try await provider.recordFocus([
            InteractionFocusInterval(app: "app:/Applications/Terminal.app", startedAtMs: now,
                                     endedAtMs: now + 200, endedBy: "switch")
        ])
        await provider.shutdown()
    }
}
