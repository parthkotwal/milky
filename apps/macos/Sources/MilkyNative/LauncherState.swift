import Foundation
import Observation
import os

@MainActor
public protocol AppOpening {
    func open(_ result: AppResult) async throws
    func performSecondary(_ action: ResultMenuAction, on result: AppResult) async throws
}

public extension AppOpening {
    func performSecondary(_ action: ResultMenuAction, on result: AppResult) async throws {
        throw CocoaError(.featureUnsupported)
    }
}

@MainActor @Observable
public final class LauncherState {
    private static let usageLogger = Logger(subsystem: "com.milky", category: "usage")
    public private(set) var query = ""
    public private(set) var results: [AppResult] = []
    public private(set) var selectedID: String?
    public private(set) var isSearching = false
    public private(set) var showsProgress = false
    public private(set) var isOpening = false
    public private(set) var errorMessage: String?
    public private(set) var statusMessage: String?
    public private(set) var inspectedResultID: String?
    public private(set) var actionMenuResultID: String?
    public private(set) var selectedActionIndex = 0
    public var onOpened: (() -> Void)?
    public var onQueryChanged: ((String) -> Void)?
    public var onRendered: ((String, Int) -> Void)?
    public var onHighlight: ((String, String, Int) -> Void)?
    public var onTabComplete: ((String, String) -> Void)?

    private let provider: any SearchProvider
    private let opener: any AppOpening
    private var request: Task<Void, Never>?
    private var progress: Task<Void, Never>?
    private var generation = 0
    private var active = true
    private var currentSearchID: String?
    private var renderedCounts: [String: Int] = [:]

    public init(provider: any SearchProvider, opener: any AppOpening) {
        self.provider = provider
        self.opener = opener
    }

    public var selected: AppResult? { results.first { $0.id == selectedID } }
    public var activeSearchID: String? { currentSearchID }
    public var canOpen: Bool { active && !isSearching && !isOpening && selected != nil }
    public var inspectedResult: AppResult? {
        guard let inspectedResultID else { return nil }
        return results.first { $0.id == inspectedResultID && $0.kind == .file }
    }
    public var menuActions: [ResultMenuAction] {
        guard let selected else { return [] }
        switch selected.kind {
        case .file: return [.open, .inspect, .reveal, .copyLocation]
        case .app, .folder: return [.open, .reveal, .copyLocation]
        case .setting: return [.open, .copyLocation]
        }
    }
    public var inlineSuggestion: AppResult? {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, let result = results.first,
              result.title.compare(query, options: [.caseInsensitive, .diacriticInsensitive]) != .orderedSame else {
            return nil
        }
        return result
    }

    @discardableResult
    public func acceptInlineSuggestion() -> Bool {
        guard !isSearching, !isOpening, let suggestion = inlineSuggestion else { return false }
        if let currentSearchID { onTabComplete?(currentSearchID, suggestion.id) }
        setQuery(suggestion.title)
        return true
    }

    public func setQuery(_ value: String) {
        guard !isOpening else { return }
        let changed = value != query
        query = value
        if changed { onQueryChanged?(value) }
        statusMessage = nil
        search(resetSelection: changed)
    }

    public func refresh() { search(resetSelection: false, allowEmpty: errorMessage != nil) }

    public func showProviderError(_ error: any Error) {
        // A startup failure must not overwrite a query already being handled.
        guard active, query.isEmpty else { return }
        errorMessage = error.localizedDescription
    }

    private func search(resetSelection: Bool, allowEmpty: Bool = false) {
        closeOverlays()
        generation += 1
        let token = generation
        request?.cancel()
        progress?.cancel()
        errorMessage = nil
        showsProgress = false
        guard active, allowEmpty || !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            results = []; selectedID = nil; isSearching = false
            return
        }
        isSearching = true
        let submitted = query
        progress = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled, let self, self.generation == token else { return }
            self.showsProgress = true
        }
        request = Task { [weak self, provider] in
            do {
                let response = try await provider.searchResponse(query: submitted)
                guard let self, self.active, self.generation == token else { return }
                self.currentSearchID = response.searchID
                let values = response.results
                self.results = values
                if resetSelection || !values.contains(where: { $0.id == self.selectedID }) {
                    self.selectedID = values.first?.id
                }
                self.emitHighlightIfPossible()
                self.finishSearch()
            } catch {
                guard let self, self.active, self.generation == token else { return }
                self.results = []; self.selectedID = nil
                self.errorMessage = error.localizedDescription
                self.finishSearch()
            }
        }
    }

    private func finishSearch() {
        isSearching = false
        showsProgress = false
        progress?.cancel()
    }

    public func select(_ id: String) {
        guard !isSearching, !isOpening, results.contains(where: { $0.id == id }) else { return }
        let changed = selectedID != id
        if changed {
            closeOverlays()
            statusMessage = nil
        }
        selectedID = id
        if changed { emitHighlightIfPossible() }
    }

    public func moveSelection(_ offset: Int) {
        if actionMenuResultID != nil {
            let actions = menuActions
            guard !actions.isEmpty else { return }
            selectedActionIndex = min(max(selectedActionIndex + offset, 0), actions.count - 1)
            return
        }
        if inspectedResultID != nil { return }
        guard !isSearching, !isOpening, !results.isEmpty else { return }
        let current = results.firstIndex { $0.id == selectedID } ?? 0
        selectedID = results[min(max(current + offset, 0), results.count - 1)].id
        emitHighlightIfPossible()
    }

    public func openSelected() async {
        if actionMenuResultID != nil {
            await performSelectedMenuAction()
            return
        }
        guard canOpen, let result = selected else { return }
        let eventInfo = actionInfo(for: result)
        isOpening = true
        errorMessage = nil
        statusMessage = nil
        let token = generation
        do {
            try await opener.open(result)
            recordAction("open", info: eventInfo, succeeded: true)
            if active, generation == token { onOpened?() }
        } catch {
            recordAction("open", info: eventInfo, succeeded: false)
            if active, generation == token {
                errorMessage = "Couldn’t open \(result.title). \(error.localizedDescription)"
            }
        }
        isOpening = false
    }

    public func toggleInspection() {
        guard canOpen, selected?.kind == .file else { return }
        if inspectedResultID == selectedID {
            inspectedResultID = nil
        } else {
            if let selected, let info = actionInfo(for: selected) { recordAction("inspect", info: info, succeeded: true) }
            actionMenuResultID = nil
            inspectedResultID = selectedID
        }
    }

    public func toggleActionMenu() {
        guard canOpen else { return }
        if actionMenuResultID == selectedID {
            actionMenuResultID = nil
        } else {
            if let selected, let info = actionInfo(for: selected) { recordAction("menu", info: info, succeeded: true) }
            inspectedResultID = nil
            actionMenuResultID = selectedID
            selectedActionIndex = 0
        }
    }

    @discardableResult
    public func closeOverlay() -> Bool {
        guard inspectedResultID != nil || actionMenuResultID != nil else { return false }
        closeOverlays()
        return true
    }

    private func closeOverlays() {
        inspectedResultID = nil
        actionMenuResultID = nil
        selectedActionIndex = 0
    }

    public func chooseMenuAction(_ action: ResultMenuAction) async {
        guard actionMenuResultID == selectedID, menuActions.contains(action), canOpen,
              let result = selected else { return }
        actionMenuResultID = nil
        switch action {
        case .open:
            await openSelected()
        case .inspect:
            inspectedResultID = result.id
            if let info = actionInfo(for: result) { recordAction("inspect", info: info, succeeded: true) }
        case .reveal, .copyLocation:
            let eventInfo = actionInfo(for: result)
            isOpening = true
            errorMessage = nil
            statusMessage = nil
            let token = generation
            do {
                try await opener.performSecondary(action, on: result)
                recordAction(action == .reveal ? "reveal" : "copy_path", info: eventInfo, succeeded: true)
                if active, generation == token {
                    if action == .reveal { onOpened?() }
                    else { statusMessage = result.kind == .setting ? "Link copied" : "Path copied" }
                }
            } catch {
                recordAction(action == .reveal ? "reveal" : "copy_path", info: eventInfo, succeeded: false)
                if active, generation == token {
                    errorMessage = "Couldn’t \(action.title(for: result).lowercased()). \(error.localizedDescription)"
                }
            }
            isOpening = false
        }
    }

    public func performSelectedMenuAction() async {
        guard actionMenuResultID != nil, menuActions.indices.contains(selectedActionIndex) else { return }
        await chooseMenuAction(menuActions[selectedActionIndex])
    }

    private func actionInfo(for result: AppResult) -> (searchID: String, resultID: String, index: Int)? {
        guard let currentSearchID, let index = results.firstIndex(where: { $0.id == result.id }) else { return nil }
        return (currentSearchID, result.id, index)
    }
    private func recordAction(_ action: String, info: (searchID: String, resultID: String, index: Int)?, succeeded: Bool) {
        guard let info else { return }
        onAction?(action, info.searchID, info.resultID, info.index, succeeded)
    }
    public var onAction: ((String, String, String, Int, Bool) -> Void)?
    public func recordRendered(searchID: String, visible: Int) {
        guard visible > renderedCounts[searchID, default: 0] else { return }
        renderedCounts[searchID] = visible
        onRendered?(searchID, visible)
    }
    private func emitHighlightIfPossible() {
        guard let currentSearchID, let selectedID,
              let index = results.firstIndex(where: { $0.id == selectedID }) else { return }
        onHighlight?(currentSearchID, selectedID, index)
    }

    public func dismiss() {
        active = false
        closeOverlays()
        generation += 1
        request?.cancel(); progress?.cancel()
        isSearching = false; showsProgress = false
        currentSearchID = nil
        renderedCounts = [:]
    }

    @discardableResult
    public func beginSession() -> String? {
        active = true
        errorMessage = nil
        statusMessage = nil
        closeOverlays()
        generation += 1
        request?.cancel(); progress?.cancel()
        isSearching = false; showsProgress = false
        let queryToSelect = query.isEmpty ? nil : query
        if queryToSelect != nil { search(resetSelection: false) }
        return queryToSelect
    }
}
