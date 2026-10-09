import AppKit
import SwiftUI

public enum PanelDesign: String, CaseIterable, Sendable {
    case balanced
    case compact
    case guided
    case hybrid
}

enum LauncherTheme {
    static let rowHeight: CGFloat = 54
    static let inset: CGFloat = 16
    static let corner: CGFloat = 14
    static let title = Font.system(size: 14, weight: .medium)
    static let subtitle = Font.system(size: 12)
    static let hint = Font.system(size: 11, weight: .medium)
    static func queryWidth(for query: String) -> CGFloat {
        let measured = (query as NSString).size(withAttributes: [.font: NSFont.systemFont(ofSize: 24)]).width + 5
        return min(max(measured, 18), 280)
    }
    static let secondaryTextColor = NSColor(name: nil) { appearance in
        var color = NSColor.labelColor
        appearance.performAsCurrentDrawingAppearance {
            color = NSColor.labelColor.withAlphaComponent(0.72)
        }
        return color
    }
    static let secondary = Color(nsColor: secondaryTextColor)
}

public struct LauncherView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Bindable var state: LauncherState
    let dismiss: () -> Void
    let fixtures: Bool
    let design: PanelDesign

    public init(state: LauncherState, fixtures: Bool = true, design: PanelDesign = .hybrid,
                dismiss: @escaping () -> Void) {
        self.state = state
        self.fixtures = fixtures
        self.design = design
        self.dismiss = dismiss
    }

    public var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: design == .compact ? 9 : 12) {
                Image(systemName: "magnifyingglass").font(.system(size: 22)).foregroundStyle(LauncherTheme.secondary)
                QueryField(value: Binding(get: { state.query }, set: { state.setQuery($0) }),
                           enabled: !state.isOpening, up: { state.moveSelection(-1) },
                           down: { state.moveSelection(1) }, submit: open, dismiss: dismissOrClose,
                           acceptSuggestion: { state.acceptInlineSuggestion() })
                    .frame(width: state.query.isEmpty ? nil : LauncherTheme.queryWidth(for: state.query),
                           height: design == .compact ? 34 : 40)
                    .accessibilityLabel("Search your Mac")
                if design != .guided, let suggestion = state.inlineSuggestion {
                    suggestionButton(suggestion, inline: true)
                }
                Spacer(minLength: 0)
                if design == .balanced || design == .hybrid, let suggestion = state.inlineSuggestion {
                    ResultIcon(result: suggestion).frame(width: 32, height: 32)
                        .transition(reduceMotion ? .identity : .opacity)
                }
                if state.showsProgress || state.isOpening {
                    ProgressView().controlSize(.small).accessibilityLabel(state.isOpening ? "Opening application" : "Searching")
                }
            }
            .padding(.horizontal, LauncherTheme.inset)
            .padding(.vertical, design == .compact ? 10 : LauncherTheme.inset)
            if design == .guided, let suggestion = state.inlineSuggestion {
                HStack(spacing: 10) {
                    Text("TOP MATCH")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .tracking(0.8)
                        .foregroundStyle(LauncherTheme.secondary)
                        .frame(width: 84, alignment: .leading)
                    ResultIcon(result: suggestion).frame(width: 22, height: 22)
                    suggestionButton(suggestion, inline: false)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, LauncherTheme.inset)
                .padding(.bottom, 12)
            }
            Divider()
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .opacity(state.showsProgress && !state.results.isEmpty ? 0.68 : 1)
                .animation(reduceMotion ? nil : .easeInOut(duration: 0.14), value: state.results)
                .animation(reduceMotion ? nil : .easeInOut(duration: 0.14), value: state.showsProgress)
                .animation(reduceMotion ? nil : .easeInOut(duration: 0.12), value: state.inlineSuggestion?.id)
            if let error = state.errorMessage {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "exclamationmark.triangle")
                    Text(error).font(LauncherTheme.subtitle).textSelection(.enabled)
                    Spacer(minLength: 0)
                    Button("Retry") {
                        if state.results.isEmpty { state.refresh() } else { open() }
                    }.disabled(state.isOpening || state.isSearching)
                }
                .padding(LauncherTheme.inset)
                .accessibilityElement(children: .contain)
            }
            if let status = state.statusMessage {
                HStack {
                    Image(systemName: "checkmark.circle")
                    Text(status)
                    Spacer()
                }
                .font(LauncherTheme.subtitle)
                .foregroundStyle(LauncherTheme.secondary)
                .padding(.horizontal, LauncherTheme.inset)
                .padding(.vertical, 7)
            }
            Divider()
            HStack(spacing: design == .compact ? 10 : 14) {
                if fixtures {
                    Label("Fixtures", systemImage: "testtube.2")
                        .help("Development fixtures only. No Rust search or usage recording.")
                } else {
                    Label("Local Results", systemImage: "square.grid.2x2")
                }
                Spacer()
                if state.actionMenuResultID != nil {
                    Text("↑↓ Choose")
                    Text("↵ Run")
                    Text("esc Back")
                } else if state.inspectedResultID != nil {
                    Text("↵ Open")
                    Text("esc Back")
                } else {
                    if design != .compact { Text("↑↓ Navigate") }
                    if state.selected?.kind == .file {
                        Button("⌘P Inspect") { state.toggleInspection() }
                            .buttonStyle(.plain).disabled(!state.canOpen)
                    }
                    if state.selected != nil {
                        Button("⌘K Actions") { state.toggleActionMenu() }
                            .buttonStyle(.plain).disabled(!state.canOpen)
                    }
                    Text("↵ Open").foregroundStyle(state.canOpen ? Color.primary : LauncherTheme.secondary)
                    Text("esc Close")
                }
            }
            .font(LauncherTheme.hint).foregroundStyle(LauncherTheme.secondary)
            .padding(.horizontal, LauncherTheme.inset)
            .padding(.vertical, design == .compact ? 8 : 11)
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: LauncherTheme.corner))
        .overlay(RoundedRectangle(cornerRadius: LauncherTheme.corner).strokeBorder(.primary.opacity(0.12)))
        .onChange(of: state.errorMessage) { _, message in
            if let message { announce(message) }
        }
        .onChange(of: state.isSearching) { _, searching in
            if !searching, state.results.isEmpty, state.errorMessage == nil, !state.query.isEmpty {
                announce("No matching results")
            }
        }
    }

    private func suggestionButton(_ suggestion: AppResult, inline: Bool) -> some View {
        Button {
            state.acceptInlineSuggestion()
        } label: {
            HStack(spacing: 7) {
                Text(inline ? "— \(suggestion.title)" : suggestion.title)
                    .font(.system(size: inline ? 14 : 13, weight: inline ? .regular : .medium))
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text("tab")
                    .font(.system(size: 10, weight: .medium))
                    .padding(.horizontal, 5).padding(.vertical, 3)
                    .background(.primary.opacity(0.09), in: RoundedRectangle(cornerRadius: 5))
            }
            .foregroundStyle(inline ? LauncherTheme.secondary : Color.primary)
            .padding(.horizontal, inline ? 9 : 0)
            .padding(.vertical, inline ? 6 : 0)
            .background(inline ? Color.primary.opacity(0.055) : .clear,
                        in: Capsule())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: inline ? 235 : 300, alignment: .leading)
        .disabled(state.isSearching || state.isOpening)
        .opacity(state.isSearching || state.isOpening ? 0.5 : 1)
        .help("Complete with \(suggestion.title) (Tab)")
        .accessibilityLabel("Complete query with \(suggestion.title)")
        .accessibilityHint("Press Tab to replace the query with this top result")
        .accessibilityAddTraits(.isButton)
        .transition(reduceMotion ? .identity : .opacity)
    }

    @ViewBuilder private var content: some View {
        if let result = state.inspectedResult {
            FileInspectionView(result: result) { state.closeOverlay() }
        } else if state.actionMenuResultID != nil, let result = state.selected {
            actionMenu(for: result)
        } else if state.results.isEmpty {
            if state.isSearching && !state.showsProgress {
                Color.clear
            } else {
                VStack(spacing: 10) {
                    Image(systemName: emptySymbol)
                        .font(.system(size: 30)).foregroundStyle(LauncherTheme.secondary)
                    Text(emptyTitle).font(.system(size: 16, weight: .medium))
                    Text(emptySubtitle)
                        .font(LauncherTheme.subtitle).foregroundStyle(LauncherTheme.secondary)
                }
                .padding(24)
                .transition(.opacity)
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(state.results) { result in
                            row(result).id(result.id).transition(.opacity)
                        }
                    }.padding(8)
                }
                .onChange(of: state.selectedID, initial: true) { _, id in
                    if let id { proxy.scrollTo(id, anchor: .center) }
                    if let selected = state.selected,
                       let index = state.results.firstIndex(where: { $0.id == selected.id }) {
                        let destination = selected.accessibilityDestination
                        announce("\(selected.title), \(selected.subtitle), \(destination), result \(index + 1) of \(state.results.count)")
                    }
                }
                .onChange(of: state.errorMessage) { _, _ in
                    // Error feedback changes the viewport height; retain the selected row.
                    DispatchQueue.main.async {
                        if let id = state.selectedID { proxy.scrollTo(id, anchor: .center) }
                    }
                }
                .transition(.opacity)
            }
        }
    }

    private func actionMenu(for result: AppResult) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                ResultIcon(result: result)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Actions for \(result.title)")
                        .font(.system(size: 16, weight: .semibold))
                        .lineLimit(1)
                    Text(result.subtitle).font(LauncherTheme.subtitle)
                        .foregroundStyle(LauncherTheme.secondary)
                        .lineLimit(1).truncationMode(.middle)
                }
                Spacer()
                Button("Back") { state.closeOverlay() }
                    .buttonStyle(.plain)
            }
            .padding(LauncherTheme.inset)
            Divider()
            ForEach(Array(state.menuActions.enumerated()), id: \.element) { index, action in
                let highlighted = state.selectedActionIndex == index
                Button {
                    Task { await state.chooseMenuAction(action) }
                } label: {
                    HStack {
                        Image(systemName: actionSymbol(action))
                            .frame(width: 22)
                        Text(action.title(for: result))
                        Spacer()
                        if highlighted { Image(systemName: "return") }
                    }
                    .font(LauncherTheme.title)
                    .padding(.horizontal, 14)
                    .frame(height: 46)
                    .background(highlighted ? Color.accentColor.opacity(0.13) : .clear,
                                in: RoundedRectangle(cornerRadius: 7))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(highlighted ? .isSelected : [])
            }
            .padding(.horizontal, 8)
            Spacer(minLength: 0)
        }
    }

    private func actionSymbol(_ action: ResultMenuAction) -> String {
        switch action {
        case .open: "arrow.up.right.square"
        case .inspect: "eye"
        case .reveal: "folder"
        case .copyLocation: "doc.on.doc"
        }
    }

    private var emptyTitle: String {
        if state.errorMessage != nil && state.results.isEmpty { return "Search unavailable" }
        if state.query.isEmpty { return fixtures ? "Search fixture results" : "Search your Mac" }
        if state.isSearching { return state.showsProgress ? "Searching…" : "" }
        if state.errorMessage != nil { return "Search unavailable" }
        return "No matching results"
    }

    private var emptySymbol: String {
        if state.errorMessage != nil { return "exclamationmark.magnifyingglass" }
        if state.isSearching { return "magnifyingglass" }
        return state.query.isEmpty ? "app.dashed" : "magnifyingglass"
    }

    private var emptySubtitle: String {
        if state.errorMessage != nil { return "Change your query or retry below." }
        if state.isSearching { return "Searching local results." }
        if state.query.isEmpty {
            return fixtures
                ? "Try Safari, Calendar, or IDLE. Type all to inspect every fixture."
                : "Type an app, file, folder, or setting name."
        }
        return "Try another name or path."
    }

    private func row(_ result: AppResult) -> some View {
        let selected = state.selectedID == result.id
        return HStack(spacing: design == .compact ? 9 : 12) {
            ResultIcon(result: result).frame(width: design == .compact ? 26 : 32,
                                             height: design == .compact ? 26 : 32)
            VStack(alignment: .leading, spacing: design == .compact ? 1 : 3) {
                Text(result.title).font(LauncherTheme.title).lineLimit(1)
                Text(result.subtitle).font(LauncherTheme.subtitle)
                    .foregroundStyle(LauncherTheme.secondary).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 8)
            if design != .balanced {
                Text(kindLabel(result.kind))
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(LauncherTheme.secondary)
                    .padding(.horizontal, 7).padding(.vertical, 4)
                    .background(.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 5))
            }
            if selected { Image(systemName: "return").font(LauncherTheme.subtitle).foregroundStyle(LauncherTheme.secondary) }
        }
        .padding(.horizontal, 12)
        .frame(height: design == .compact ? 44 : design == .guided ? 58 : LauncherTheme.rowHeight)
        .background(selected ? Color.accentColor.opacity(0.13) : .clear, in: RoundedRectangle(cornerRadius: 7))
        .overlay(alignment: .leading) {
            if selected { RoundedRectangle(cornerRadius: 2).fill(LauncherTheme.secondary).frame(width: 3, height: 24) }
        }
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { state.select(result.id); open() }
        .onTapGesture { state.select(result.id) }
        .help(result.title + " — " + result.accessibilityDestination)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(result.title), \(result.subtitle), \(result.accessibilityDestination), \(result.kind.rawValue)")
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : [.isButton])
        .accessibilityAction { state.select(result.id); open() }
        .accessibilityAction(named: "Select") { state.select(result.id) }
    }

    private func kindLabel(_ kind: ResultKind) -> String {
        switch kind {
        case .app: "App"
        case .setting: "Setting"
        case .folder: "Folder"
        case .file: "File"
        }
    }

    private func announce(_ message: String) {
        guard NSWorkspace.shared.isVoiceOverEnabled else { return }
        NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested,
            userInfo: [.announcement: message, .priority: NSAccessibilityPriorityLevel.medium.rawValue])
    }

    private func open() { Task { await state.openSelected() } }
    private func dismissOrClose() {
        if !state.closeOverlay() { dismiss() }
    }
}

private struct ResultIcon: View {
    let result: AppResult
    @State private var image: NSImage?
    var body: some View {
        Group {
            if let image { Image(nsImage: image).resizable() }
            else {
                let symbol: String = switch result.kind {
                case .app: "app.dashed"
                case .setting: "slider.horizontal.3"
                case .folder: "folder.fill"
                case .file: "doc.fill"
                }
                Image(systemName: symbol).resizable().padding(5).foregroundStyle(LauncherTheme.secondary)
            }
        }
        .frame(width: 32, height: 32).accessibilityHidden(true)
        .task(id: result.id) {
            if let data = await IconCache.shared.data(for: result), !Task.isCancelled {
                image = NSImage(data: data)
            }
        }
    }
}

/// Uses the native field editor so arrows/Return/Escape respect marked IME text.
private struct QueryField: NSViewRepresentable {
    @Binding var value: String
    let enabled: Bool
    let up: () -> Void
    let down: () -> Void
    let submit: () -> Void
    let dismiss: () -> Void
    let acceptSuggestion: () -> Bool

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSTextField {
        let field = NSTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 24)
        field.placeholderString = "Search your Mac…"
        field.setAccessibilityLabel("Search your Mac")
        field.setAccessibilityHelp("Type a destination name. Tab completes the top suggestion. Up and Down select a result or action. Return opens or runs it. Command K shows actions, Command P inspects a file, and Escape goes back or closes.")
        field.delegate = context.coordinator
        return field
    }
    func updateNSView(_ field: NSTextField, context: Context) {
        context.coordinator.parent = self
        if field.stringValue != value { field.stringValue = value }
        let restoreFocus = enabled && !field.isEnabled
        field.isEnabled = enabled
        if restoreFocus, field.window?.isKeyWindow == true {
            field.window?.makeFirstResponder(field)
        }
    }
    final class Coordinator: NSObject, NSTextFieldDelegate {
        var parent: QueryField
        init(_ parent: QueryField) { self.parent = parent }
        func controlTextDidChange(_ notification: Notification) {
            guard let field = notification.object as? NSTextField else { return }
            parent.value = field.stringValue
        }
        func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
            guard !textView.hasMarkedText() else { return false }
            switch commandSelector {
            case #selector(NSResponder.moveUp(_:)): parent.up()
            case #selector(NSResponder.moveDown(_:)): parent.down()
            case #selector(NSResponder.insertNewline(_:)): parent.submit()
            case #selector(NSResponder.cancelOperation(_:)): parent.dismiss()
            case #selector(NSResponder.insertTab(_:)): return parent.acceptSuggestion()
            default: return false
            }
            return true
        }
    }
}
