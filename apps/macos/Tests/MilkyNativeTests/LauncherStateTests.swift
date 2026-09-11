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

@MainActor final class LauncherStateTests: XCTestCase {
    private let first = AppResult(id: "one", name: "IDLE", url: URL(fileURLWithPath: "/one/IDLE.app"))
    private let second = AppResult(id: "two", name: "IDLE", url: URL(fileURLWithPath: "/two/IDLE.app"))

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
        XCTAssertEqual(opener.opened, ["two"])
    }

    func testDuplicateIdentityRefreshAndNewQuerySelection() async throws {
        let provider = ControlledSearch()
        let state = LauncherState(provider: provider, opener: RecordingOpener())
        state.setQuery("idle")
        try await waitFor { await provider.contains("idle") }
        await provider.succeed("idle", [first, second])
        try await waitFor { !state.isSearching }
        state.moveSelection(1)
        XCTAssertEqual(state.selectedID, "two")
        state.moveSelection(1)
        XCTAssertEqual(state.selectedID, "two")
        state.refresh()
        try await waitFor { await provider.contains("idle") }
        XCTAssertFalse(state.canOpen)
        await provider.succeed("idle", [second, first])
        try await waitFor { !state.isSearching }
        XCTAssertEqual(state.selectedID, "two")
        state.setQuery("different")
        XCTAssertNil(state.selectedID)
        try await waitFor { await provider.contains("different") }
        await provider.succeed("different", [first, second])
        try await waitFor { !state.isSearching }
        XCTAssertEqual(state.selectedID, "one")
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

    func testNativeIconIsBoundedAndMissingIconUsesFallback() async throws {
        let url = URL(fileURLWithPath: "/System/Applications/Calculator.app")
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw XCTSkip("Calculator is unavailable on this host")
        }
        let data = await IconCache.shared.data(for: url)
        let unwrapped = try XCTUnwrap(data)
        let image = try XCTUnwrap(NSBitmapImageRep(data: unwrapped))
        XCTAssertEqual(image.pixelsWide, 64)
        XCTAssertEqual(image.pixelsHigh, 64)
        XCTAssertLessThan(unwrapped.count, 100_000)
        let missing = await IconCache.shared.data(for: URL(fileURLWithPath: "/tmp/Milky Fixtures/DoesNotExist.app"))
        XCTAssertNil(missing)
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
