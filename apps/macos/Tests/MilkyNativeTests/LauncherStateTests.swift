import AppKit
import XCTest
@testable import MilkyNative

private actor ControlledSearch: SearchProvider {
    private var pending: [String: CheckedContinuation<[AppResult], any Error>] = [:]
    func search(query: String) async throws -> [AppResult] {
        try await withCheckedThrowingContinuation { pending[query] = $0 }
    }
    func contains(_ query: String) -> Bool { pending[query] != nil }
    func succeed(_ query: String, _ results: [AppResult]) { pending.removeValue(forKey: query)?.resume(returning: results) }
    func fail(_ query: String) { pending.removeValue(forKey: query)?.resume(throwing: FixtureError.unavailable) }
}

@MainActor private final class RecordingOpener: AppOpening {
    var opened: [String] = []
    var fails = false
    func open(_ result: AppResult) async throws {
        opened.append(result.id)
        if fails { throw FixtureError.unavailable }
    }
}

private actor SelectionRecorder: SelectionEventRecording {
    private(set) var events: [SelectionEvent] = []
    func recordSelection(_ event: SelectionEvent) async throws { events.append(event) }
    func recordedEvents() -> [SelectionEvent] { events }
}

@MainActor final class LauncherStateTests: XCTestCase {
    private let first = AppResult(id: "app:/one/IDLE.app", title: "IDLE", subtitle: "one", action: .launch(URL(fileURLWithPath: "/one/IDLE.app")))
    private let second = AppResult(id: "app:/two/IDLE.app", title: "IDLE", subtitle: "two", action: .launch(URL(fileURLWithPath: "/two/IDLE.app")))

    private func waitFor(_ predicate: () async -> Bool) async throws {
        for _ in 0..<2000 {
            if await predicate() { return }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTFail("Timed out waiting for a state transition")
        throw FixtureError.unavailable
    }

    func testLateResponseCannotReplaceNewQueryOrBeOpenedDuringRefresh() async throws {
        let provider = ControlledSearch()
        let opener = RecordingOpener()
        let state = LauncherState(provider: provider, opener: opener)
        state.setQuery("old")
        try await waitFor { await provider.contains("old") }
        state.setQuery("new")
        try await waitFor { await provider.contains("new") }
        await state.openSelected()
        XCTAssertTrue(opener.opened.isEmpty)
        await provider.succeed("new", [second])
        try await waitFor { !state.isSearching }
        await provider.succeed("old", [first])
        // Wait for the cancelled provider, which deliberately ignores cancellation.
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(state.results, [second])
        await state.openSelected()
        XCTAssertEqual(opener.opened, ["app:/two/IDLE.app"])
    }

    func testDuplicateIdentityRefreshAndNewQuerySelection() async throws {
        let provider = ControlledSearch()
        let opener = RecordingOpener()
        let state = LauncherState(provider: provider, opener: opener)
        state.setQuery("idle")
        try await waitFor { await provider.contains("idle") }
        await provider.succeed("idle", [first, second])
        try await waitFor { !state.isSearching }
        state.moveSelection(1)
        XCTAssertEqual(state.selectedID, "app:/two/IDLE.app")
        state.moveSelection(1)
        XCTAssertEqual(state.selectedID, "app:/two/IDLE.app")
        state.refresh()
        try await waitFor { await provider.contains("idle") }
        XCTAssertFalse(state.canOpen)
        await provider.succeed("idle", [second, first])
        try await waitFor { !state.isSearching }
        XCTAssertEqual(state.selectedID, "app:/two/IDLE.app")
        state.setQuery("different")
        XCTAssertEqual(state.results, [second, first], "Keep the current rows visible until the new query arrives")
        XCTAssertEqual(state.selectedID, second.id, "Keep the current highlight stable during the refresh")
        XCTAssertFalse(state.canOpen, "A stale result must not launch for the new query")
        await state.openSelected()
        XCTAssertTrue(opener.opened.isEmpty)
        try await waitFor { await provider.contains("different") }
        await provider.succeed("different", [first, second])
        try await waitFor { !state.isSearching }
        XCTAssertEqual(state.selectedID, "app:/one/IDLE.app")
    }

    func testInlineSuggestionUsesTopResultAndTabCompletesOnlyAfterSearchFinishes() async throws {
        let provider = ControlledSearch()
        let state = LauncherState(provider: provider, opener: RecordingOpener())
        let visualStudioCode = AppResult(
            id: "app:/Applications/Visual Studio Code.app",
            title: "Visual Studio Code",
            subtitle: "Applications",
            action: .launch(URL(fileURLWithPath: "/Applications/Visual Studio Code.app"))
        )

        state.setQuery("vsc")
        try await waitFor { await provider.contains("vsc") }
        XCTAssertFalse(state.acceptInlineSuggestion(), "A stale or pending result must not be accepted")
        await provider.succeed("vsc", [visualStudioCode, first])
        try await waitFor { !state.isSearching }

        XCTAssertEqual(state.inlineSuggestion, visualStudioCode, "Acronym matches can suggest the ranked result")
        XCTAssertTrue(state.acceptInlineSuggestion())
        XCTAssertEqual(state.query, "Visual Studio Code")
        XCTAssertNil(state.inlineSuggestion, "An exact title match should not repeat as a suggestion")
        XCTAssertTrue(state.isSearching)
        XCTAssertFalse(state.canOpen, "Completing a query does not launch the result")
    }

    func testEmptyFailureRecoveryAndActionFailure() async throws {
        let provider = ControlledSearch()
        let opener = RecordingOpener()
        let state = LauncherState(provider: provider, opener: opener)
        state.setQuery("none")
        try await waitFor { await provider.contains("none") }
        await provider.succeed("none", [])
        try await waitFor { !state.isSearching }
        XCTAssertNil(state.errorMessage)
        XCTAssertFalse(state.canOpen)
        state.setQuery("error")
        try await waitFor { await provider.contains("error") }
        await provider.fail("error")
        try await waitFor { !state.isSearching }
        XCTAssertNotNil(state.errorMessage)
        state.refresh()
        try await waitFor { await provider.contains("error") }
        await provider.succeed("error", [first])
        try await waitFor { !state.isSearching }
        opener.fails = true
        var didOpen = false
        state.onOpened = { didOpen = true }
        await state.openSelected()
        XCTAssertNotNil(state.errorMessage)
        XCTAssertFalse(didOpen)
        XCTAssertTrue(state.canOpen)
        opener.fails = false
        await state.openSelected()
        XCTAssertTrue(didOpen)
        XCTAssertNil(state.errorMessage)
    }

    func testRecordsShownResultsAndActionOutcomeAfterAttempt() async throws {
        let provider = ControlledSearch()
        let opener = RecordingOpener()
        let recorder = SelectionRecorder()
        let state = LauncherState(provider: provider, opener: opener, eventRecorder: recorder)
        state.setQuery("idle")
        try await waitFor { await provider.contains("idle") }
        await provider.succeed("idle", [first, second])
        try await waitFor { !state.isSearching }
        state.select(second.id)

        opener.fails = true
        await state.openSelected()
        try await waitFor { await recorder.recordedEvents().count == 1 }
        let failedEvent = await recorder.recordedEvents()[0]
        XCTAssertEqual(failedEvent, SelectionEvent(query: "idle", shown: ["app:/one/IDLE.app", "app:/two/IDLE.app"], selected: "app:/two/IDLE.app", outcome: .failed))

        opener.fails = false
        await state.openSelected()
        try await waitFor { await recorder.recordedEvents().count == 2 }
        let openedEvent = await recorder.recordedEvents()[1]
        XCTAssertEqual(openedEvent.outcome, .opened)
    }

    func testNativeIconIsBoundedAndMissingIconUsesFallback() async throws {
        let url = URL(fileURLWithPath: "/System/Applications/Calculator.app")
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw XCTSkip("Calculator is unavailable on this host")
        }
        let app = AppResult(id: "app:\(url.path)", title: "Calculator", action: .launch(url))
        let data = await IconCache.shared.data(for: app)
        let unwrapped = try XCTUnwrap(data)
        let image = try XCTUnwrap(NSBitmapImageRep(data: unwrapped))
        XCTAssertEqual(image.pixelsWide, 64)
        XCTAssertEqual(image.pixelsHigh, 64)
        XCTAssertLessThan(unwrapped.count, 100_000)
        let missingURL = URL(fileURLWithPath: "/tmp/Milky Fixtures/DoesNotExist.app")
        let missingApp = AppResult(id: "app:\(missingURL.path)", title: "Missing", action: .launch(missingURL))
        let missing = await IconCache.shared.data(for: missingApp)
        XCTAssertNil(missing)
    }

    func testSettingsResultUsesItsSystemPaneIcon() async throws {
        let bundle = URL(fileURLWithPath: "/System/Library/ExtensionKit/Extensions/Wi-Fi.appex")
        guard FileManager.default.fileExists(atPath: bundle.path) else {
            throw XCTSkip("Wi-Fi settings pane is unavailable on this host")
        }
        let result = AppResult(
            id: "settings:com.apple.wifi-settings-extension#Advanced",
            kind: .setting,
            title: "Advanced",
            subtitle: "Wi-Fi",
            action: .openURL(URL(string: "x-apple.systempreferences:com.apple.wifi-settings-extension?Advanced")!)
        )
        let iconData = await IconCache.shared.data(for: result)
        let data = try XCTUnwrap(iconData)
        let image = try XCTUnwrap(NSBitmapImageRep(data: data))
        XCTAssertEqual(image.pixelsWide, 64)
        XCTAssertEqual(image.pixelsHigh, 64)
        XCTAssertLessThan(data.count, 100_000)

        let network = AppResult(
            id: "settings:com.apple.Network-Settings.extension",
            kind: .setting,
            title: "Network",
            subtitle: "System Settings",
            action: .openURL(URL(string: "x-apple.systempreferences:com.apple.Network-Settings.extension")!)
        )
        let networkIconData = await IconCache.shared.data(for: network)
        let networkData = try XCTUnwrap(networkIconData)
        XCTAssertNotEqual(data, networkData, "Each pane should resolve its own bundle artwork")
    }

    func testRemovedSelectionAndLateErrorDoNotCorruptCurrentResults() async throws {
        let provider = ControlledSearch()
        let state = LauncherState(provider: provider, opener: RecordingOpener())
        state.setQuery("late error")
        try await waitFor { await provider.contains("late error") }
        state.setQuery("idle")
        try await waitFor { await provider.contains("idle") }
        await provider.succeed("idle", [first, second])
        try await waitFor { !state.isSearching }
        state.select(second.id)
        await provider.fail("late error")
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertNil(state.errorMessage)
        XCTAssertEqual(state.selectedID, second.id)
        state.refresh()
        try await waitFor { await provider.contains("idle") }
        await provider.succeed("idle", [first])
        try await waitFor { !state.isSearching }
        XCTAssertEqual(state.selectedID, first.id)
        state.setQuery("   ")
        XCTAssertTrue(state.results.isEmpty)
        XCTAssertFalse(state.isSearching)
    }

    func testStartupFailureCanRetryWithoutTypingAndCannotOverwriteAnActiveQuery() async throws {
        let provider = ControlledSearch()
        let state = LauncherState(provider: provider, opener: RecordingOpener())
        state.showProviderError(FixtureError.unavailable)
        XCTAssertNotNil(state.errorMessage)
        state.refresh()
        try await waitFor { await provider.contains("") }
        await provider.succeed("", [])
        try await waitFor { !state.isSearching }
        XCTAssertNil(state.errorMessage)
        state.setQuery("new")
        state.showProviderError(FixtureError.unavailable)
        XCTAssertNil(state.errorMessage)
        try await waitFor { await provider.contains("new") }
        await provider.succeed("new", [first])
        try await waitFor { !state.isSearching }
    }

    func testDismissalInvalidatesPendingWorkAndReinvocationResets() async throws {
        let provider = ControlledSearch()
        let state = LauncherState(provider: provider, opener: RecordingOpener())
        state.setQuery("old")
        try await waitFor { await provider.contains("old") }
        state.dismiss()
        state.beginSession()
        await provider.succeed("old", [first])
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(state.query, "")
        XCTAssertTrue(state.results.isEmpty)
        XCTAssertFalse(state.canOpen)
        XCTAssertFalse(state.showsProgress)
    }
}
