import Foundation
import Observation

@MainActor
public protocol AppOpening {
    func open(_ result: AppResult) async throws
}

@MainActor @Observable
public final class LauncherState {
    public private(set) var query = ""
    public private(set) var results: [AppResult] = []
    public private(set) var selectedID: String?
    public private(set) var isSearching = false
    public private(set) var showsProgress = false
    public private(set) var isOpening = false
    public private(set) var errorMessage: String?
    public var onOpened: (() -> Void)?

    private let provider: any SearchProvider
    private let opener: any AppOpening
    private let eventRecorder: (any SelectionEventRecording)?
    private var request: Task<Void, Never>?
    private var progress: Task<Void, Never>?
    private var generation = 0
    private var active = true

    public init(provider: any SearchProvider, opener: any AppOpening,
                eventRecorder: (any SelectionEventRecording)? = nil) {
        self.provider = provider
        self.opener = opener
        self.eventRecorder = eventRecorder
    }

    public var selected: AppResult? { results.first { $0.id == selectedID } }
    public var canOpen: Bool { active && !isSearching && !isOpening && selected != nil }

    public func setQuery(_ value: String) {
        guard !isOpening else { return }
        let changed = value != query
        query = value
        search(resetSelection: changed)
    }

    public func refresh() { search(resetSelection: false, allowEmpty: errorMessage != nil) }

    public func showProviderError(_ error: any Error) {
        // A startup failure must not overwrite a query already being handled.
        guard active, query.isEmpty else { return }
        errorMessage = error.localizedDescription
    }

    private func search(resetSelection: Bool, allowEmpty: Bool = false) {
        generation += 1
        let token = generation
        request?.cancel()
        progress?.cancel()
        errorMessage = nil
        showsProgress = false
        if resetSelection { results = []; selectedID = nil }
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
                let values = try await provider.search(query: submitted)
                guard let self, self.active, self.generation == token else { return }
                self.results = values
                if !values.contains(where: { $0.id == self.selectedID }) {
                    self.selectedID = values.first?.id
                }
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
        selectedID = id
    }

    public func moveSelection(_ offset: Int) {
        guard !isSearching, !isOpening, !results.isEmpty else { return }
        let current = results.firstIndex { $0.id == selectedID } ?? 0
        selectedID = results[min(max(current + offset, 0), results.count - 1)].id
    }

    public func openSelected() async {
        guard canOpen, let result = selected else { return }
        let eventQuery = query
        let shownIDs = results.map(\.id)
        isOpening = true
        errorMessage = nil
        let token = generation
        do {
            try await opener.open(result)
            recordSelection(query: eventQuery, shown: shownIDs, selected: result.id, outcome: .opened)
            if active, generation == token { onOpened?() }
        } catch {
            recordSelection(query: eventQuery, shown: shownIDs, selected: result.id, outcome: .failed)
            if active, generation == token {
                errorMessage = "Couldn’t open \(result.name). \(error.localizedDescription)"
            }
        }
        isOpening = false
    }

    private func recordSelection(query: String, shown: [String], selected: String,
                                 outcome: SelectionOutcome) {
        guard let eventRecorder else { return }
        let event = SelectionEvent(query: query, shown: shown, selected: selected, outcome: outcome)
        // Durable logging is deliberately off the launcher's critical path.
        Task { try? await eventRecorder.recordSelection(event) }
    }

    public func dismiss() {
        active = false
        generation += 1
        request?.cancel(); progress?.cancel()
        isSearching = false; showsProgress = false
    }

    public func beginSession() {
        active = true
        query = ""; results = []; selectedID = nil; errorMessage = nil
        generation += 1
        request?.cancel(); progress?.cancel()
        isSearching = false; showsProgress = false
    }
}
