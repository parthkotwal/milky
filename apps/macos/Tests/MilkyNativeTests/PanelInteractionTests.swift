import XCTest
@testable import MilkyNative

private struct PanelSearch: SearchProvider {
    let results: [AppResult]
    func search(query: String) async throws -> [AppResult] { results }
    func searchResponse(query: String) async throws -> SearchResponse { SearchResponse(results: results, searchID: "panel-search") }
}

@MainActor private final class PanelOpener: AppOpening {
    var opened: [String] = []
    var secondary: [(ResultMenuAction, String)] = []
    var secondaryFails = false
    func open(_ result: AppResult) async throws { opened.append(result.id) }
    func performSecondary(_ action: ResultMenuAction, on result: AppResult) async throws {
        secondary.append((action, result.id))
        if secondaryFails { throw FixtureError.unavailable }
    }
}

@MainActor final class PanelInteractionTests: XCTestCase {
    private let file = AppResult(id: "file:/tmp/report.pdf", kind: .file,
                                 title: "report.pdf", subtitle: "/tmp",
                                 action: .open(URL(fileURLWithPath: "/tmp/report.pdf")))
    private let setting = AppResult(id: "settings:wifi", kind: .setting,
                                    title: "Wi-Fi", subtitle: "System Settings",
                                    action: .openURL(URL(string: "x-apple.systempreferences:com.apple.wifi-settings-extension")!))

    private func waitForSearch(_ state: LauncherState) async throws {
        for _ in 0..<1000 {
            if !state.isSearching { return }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTFail("Search did not finish")
    }

    func testFileActionsAndInspectionUseTheSelectedResult() async throws {
        let opener = PanelOpener()
        let state = LauncherState(provider: PanelSearch(results: [file, setting]), opener: opener)
        state.setQuery("report")
        try await waitForSearch(state)

        state.toggleActionMenu()
        XCTAssertEqual(state.menuActions, [.open, .inspect, .reveal, .copyLocation])
        XCTAssertEqual(state.actionMenuResultID, file.id)
        state.moveSelection(1)
        await state.openSelected()
        XCTAssertEqual(state.inspectedResult?.id, file.id)
        XCTAssertNil(state.actionMenuResultID)
        XCTAssertTrue(state.closeOverlay())
        XCTAssertNil(state.inspectedResult)

        state.toggleActionMenu()
        state.moveSelection(1)
        state.moveSelection(1)
        await state.openSelected()
        XCTAssertEqual(opener.secondary.last?.0, .reveal)
        XCTAssertEqual(opener.secondary.last?.1, file.id)

        state.toggleActionMenu()
        await state.chooseMenuAction(.copyLocation)
        XCTAssertEqual(state.statusMessage, "Path copied")
        XCTAssertEqual(opener.secondary.last?.0, .copyLocation)
        XCTAssertTrue(opener.opened.isEmpty)
    }

    func testQueryAndSelectionCloseOverlaysAndSettingsHaveOnlyValidActions() async throws {
        let state = LauncherState(provider: PanelSearch(results: [file, setting]), opener: PanelOpener())
        state.setQuery("mixed")
        try await waitForSearch(state)
        state.toggleInspection()
        XCTAssertNotNil(state.inspectedResult)
        state.setQuery("new")
        XCTAssertNil(state.inspectedResult)
        XCTAssertFalse(state.canOpen)
        try await waitForSearch(state)

        state.select(setting.id)
        XCTAssertEqual(state.menuActions, [.open, .copyLocation])
        state.toggleInspection()
        XCTAssertNil(state.inspectedResult)
        state.toggleActionMenu()
        XCTAssertEqual(state.actionMenuResultID, setting.id)
        XCTAssertTrue(state.closeOverlay())
        XCTAssertFalse(state.closeOverlay())
    }

    func testSecondaryResultActionsExposeActionOutcomeForSessionBatch() async throws {
        let opener = PanelOpener()
        let state = LauncherState(provider: PanelSearch(results: [file, setting]), opener: opener)
        var actions: [(String, String, String, Int, Bool)] = []
        state.onAction = { actions.append(($0, $1, $2, $3, $4)) }
        state.setQuery("report")
        try await waitForSearch(state)

        state.toggleActionMenu()
        await state.chooseMenuAction(.reveal)

        opener.secondaryFails = true
        state.toggleActionMenu()
        await state.chooseMenuAction(.copyLocation)
        XCTAssertEqual(actions.map { $0.0 }, ["menu", "reveal", "menu", "copy_path"])
        XCTAssertEqual(actions[1].1, "panel-search")
        XCTAssertEqual(actions[1].2, file.id)
        XCTAssertEqual(actions[1].3, 0)
        XCTAssertTrue(actions[1].4)
        XCTAssertFalse(actions[3].4)
        XCTAssertNotNil(state.errorMessage)
    }

    func testActionFeedbackWorksWhileSessionEventsAreCollected() async throws {
        let opener = PanelOpener()
        let state = LauncherState(provider: PanelSearch(results: [setting]), opener: opener)
        var action: (String, Bool)?
        state.onAction = { action = ($0, $4) }
        state.setQuery("wifi")
        try await waitForSearch(state)

        state.toggleActionMenu()
        await state.chooseMenuAction(.copyLocation)
        XCTAssertEqual(state.statusMessage, "Link copied")
        XCTAssertNil(state.errorMessage)
        XCTAssertEqual(action?.0, "copy_path")
        XCTAssertEqual(action?.1, true)
    }
}
