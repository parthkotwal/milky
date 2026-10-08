import AppKit
import SwiftUI

enum LauncherTheme {
    static let rowHeight: CGFloat = 54
    static let inset: CGFloat = 16
    static let corner: CGFloat = 14
    static let title = Font.system(size: 14, weight: .medium)
    static let subtitle = Font.system(size: 12)
    static let hint = Font.system(size: 11, weight: .medium)
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
    @Bindable var state: LauncherState
    let dismiss: () -> Void
    let fixtures: Bool

    public init(state: LauncherState, fixtures: Bool = true, dismiss: @escaping () -> Void) {
        self.state = state
        self.fixtures = fixtures
        self.dismiss = dismiss
    }

    public var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass").font(.system(size: 22)).foregroundStyle(LauncherTheme.secondary)
                QueryField(value: Binding(get: { state.query }, set: { state.setQuery($0) }),
                           enabled: !state.isOpening, up: { state.moveSelection(-1) },
                           down: { state.moveSelection(1) }, submit: open, dismiss: dismiss)
                    .frame(height: 40)
                    .accessibilityLabel("Search apps and settings")
                if state.showsProgress || state.isOpening {
                    ProgressView().controlSize(.small).accessibilityLabel(state.isOpening ? "Opening application" : "Searching")
                }
            }
            .padding(LauncherTheme.inset)
            Divider()
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
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
            Divider()
            HStack(spacing: 14) {
                if fixtures {
                    Label("Fixtures", systemImage: "testtube.2")
                        .help("Development fixtures only. No Rust search or usage recording.")
                } else {
                    Label("Apps & Settings", systemImage: "square.grid.2x2")
                }
                Spacer()
                Text("↑↓ Navigate")
                Text("↵ Open").foregroundStyle(state.canOpen ? Color.primary : LauncherTheme.secondary)
                Text("esc Close")
            }
            .font(LauncherTheme.hint).foregroundStyle(LauncherTheme.secondary)
            .padding(.horizontal, LauncherTheme.inset).padding(.vertical, 11)
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

    @ViewBuilder private var content: some View {
        if state.results.isEmpty {
            if state.isSearching && !state.showsProgress {
                Color.clear
            } else {
                VStack(spacing: 10) {
                    Image(systemName: emptySymbol)
                        .font(.system(size: 30)).foregroundStyle(LauncherTheme.secondary)
                    Text(emptyTitle).font(.system(size: 16, weight: .medium))
                    Text(emptySubtitle)
                        .font(LauncherTheme.subtitle).foregroundStyle(LauncherTheme.secondary)
                }.padding(24)
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(state.results) { result in
                            row(result).id(result.id)
                        }
                    }.padding(8)
                }
                .onChange(of: state.selectedID, initial: true) { _, id in
                    if let id { proxy.scrollTo(id, anchor: .center) }
                    if let selected = state.selected,
                       let index = state.results.firstIndex(where: { $0.id == selected.id }) {
                        let destination = selected.kind == .app ? selected.accessibilityDestination : selected.kind.rawValue
                        announce("\(selected.title), \(selected.subtitle), \(destination), result \(index + 1) of \(state.results.count)")
                    }
                }
                .onChange(of: state.errorMessage) { _, _ in
                    // Error feedback changes the viewport height; retain the selected row.
                    DispatchQueue.main.async {
                        if let id = state.selectedID { proxy.scrollTo(id, anchor: .center) }
                    }
                }
            }
        }
    }

    private var emptyTitle: String {
        if state.errorMessage != nil && state.results.isEmpty { return "Search unavailable" }
        if state.query.isEmpty { return fixtures ? "Search fixture results" : "Search apps and settings" }
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
        if state.isSearching { return "Looking through applications." }
        if state.query.isEmpty {
            return fixtures
                ? "Try Safari, Calendar, or IDLE. Type all to inspect every fixture."
                : "Type an app or setting name to get started."
        }
        return "Try another application name."
    }

    private func row(_ result: AppResult) -> some View {
        let selected = state.selectedID == result.id
        return HStack(spacing: 12) {
            ResultIcon(result: result)
            VStack(alignment: .leading, spacing: 3) {
                Text(result.title).font(LauncherTheme.title).lineLimit(1)
                Text(result.subtitle).font(LauncherTheme.subtitle)
                    .foregroundStyle(LauncherTheme.secondary).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 8)
            if selected { Image(systemName: "return").font(LauncherTheme.subtitle).foregroundStyle(LauncherTheme.secondary) }
        }
        .padding(.horizontal, 12).frame(height: LauncherTheme.rowHeight)
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

    private func announce(_ message: String) {
        guard NSWorkspace.shared.isVoiceOverEnabled else { return }
        NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested,
            userInfo: [.announcement: message, .priority: NSAccessibilityPriorityLevel.medium.rawValue])
    }

    private func open() { Task { await state.openSelected() } }
}

private struct ResultIcon: View {
    let result: AppResult
    @State private var image: NSImage?
    var body: some View {
        Group {
            if let image { Image(nsImage: image).resizable() }
            else { Image(systemName: result.kind == .setting ? "slider.horizontal.3" : "app.dashed").resizable().padding(5).foregroundStyle(LauncherTheme.secondary) }
        }
        .frame(width: 32, height: 32).accessibilityHidden(true)
        .task(id: result.id) {
            guard let url = result.launchURL else { image = nil; return }
            if let data = await IconCache.shared.data(for: url), !Task.isCancelled {
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

    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSTextField {
        let field = NSTextField()
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 24)
        field.placeholderString = "Search apps and settings…"
        field.setAccessibilityLabel("Search apps and settings")
        field.setAccessibilityHelp("Type an app or setting name. Use Up and Down to select a result, Return to open, and Escape to close.")
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
            default: return false
            }
            return true
        }
    }
}
