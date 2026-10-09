import XCTest
@testable import MilkyRust

@MainActor final class RustSearchProviderTests: XCTestCase {
    func testRejectsUnsupportedABIBeforeEngineCreation() throws {
        XCTAssertNoThrow(try RustSearchProvider.validateVersion(4))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(3))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(1))
        XCTAssertThrowsError(try RustSearchProvider.validateVersion(2))
    }

    func testRealEngineFindsInstalledAppsAndPreservesResultsAfterOtherRequests() async throws {
        let provider = RustSearchProvider(limit: 5)
        let results = try await provider.search(query: "term")
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
        let data = Data(#"{"kind":"search","query":"idle","results":[{"id":"app:/第二/IDLE.app","kind":"app","title":"IDLE","subtitle":"第二","action":{"type":"launch","path":"/第二/IDLE.app"},"match_kind":"exact","future":42},{"id":"app:/first/IDLE.app","kind":"app","title":"IDLE","subtitle":"first","action":{"type":"launch","path":"/first/IDLE.app"},"match_kind":"prefix"}]}"#.utf8)
        let values = try RustSearchProvider.decode(data, expectedQuery: "idle")
        XCTAssertEqual(values.map(\.id), ["app:/第二/IDLE.app", "app:/first/IDLE.app"])
        XCTAssertEqual(values.map(\.name), ["IDLE", "IDLE"])
        XCTAssertEqual(values.map(\.subtitle), ["第二", "first"])
    }

    func testDecodeSettingsAndDiscriminatedActions() throws {
        let data = Data(#"{"kind":"search","query":"wifi","results":[{"id":"settings:com.apple.wifi#Advanced","kind":"setting","title":"Wi-Fi MAC Address","subtitle":"Wi-Fi","action":{"type":"open_url","url":"x-apple.systempreferences:com.apple.wifi?Advanced"},"match_kind":"prefix"}]}"#.utf8)
        let result = try XCTUnwrap(RustSearchProvider.decode(data, expectedQuery: "wifi").first)
        XCTAssertEqual(result.kind, .setting)
        XCTAssertEqual(result.action, .openURL(URL(string: "x-apple.systempreferences:com.apple.wifi?Advanced")!))
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

    func testSelectionAcknowledgementAndErrorsAreValidated() throws {
        XCTAssertNoThrow(try RustSearchProvider.validateRecorded(Data(#"{"kind":"recorded"}"#.utf8)))
        XCTAssertThrowsError(try RustSearchProvider.validateRecorded(Data(#"{"kind":"error","reason":"bad_request","message":"selected must be shown"}"#.utf8))) { error in
            guard case BridgeError.engine(let reason, _) = error else { return XCTFail("Expected engine error") }
            XCTAssertEqual(reason, "bad_request")
        }
        XCTAssertThrowsError(try RustSearchProvider.validateRecorded(Data(#"{"kind":"search","query":"x","results":[]}"#.utf8)))
        XCTAssertThrowsError(try RustSearchProvider.validateRecorded(Data(#"{"kind":"error"}"#.utf8)))
    }
}
