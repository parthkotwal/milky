import AppKit
import SwiftUI

private enum Theme {
    static let rowHeight: CGFloat = 54
    static let inset: CGFloat = 16
    static let corner: CGFloat = 14
    static let title = Font.system(size: 14, weight: .medium)
    static let subtitle = Font.system(size: 12)
    static let hint = Font.system(size: 11, weight: .medium)
    static let secondary = Color.primary.opacity(0.72)
}

public struct LauncherView: View {
    @Bindable var state: LauncherState
    let dismiss: () -> Void

    public init(state: LauncherState, dismiss: @escaping () -> Void) {
        self.state = state
        self.dismiss = dismiss
    }

    public var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass").font(.system(size: 22)).foregroundStyle(Theme.secondary)
                QueryField(value: Binding(get: { state.query }, set: { state.setQuery($0) }),
                           enabled: !state.isOpening, up: { state.moveSelection(-1) },
                           down: { state.moveSelection(1) }, submit: open, dismiss: dismiss)
                    .frame(height: 40)
                    .accessibilityLabel("Search applications")
                if state.showsProgress || state.isOpening {
                    ProgressView().controlSize(.small).accessibilityLabel(state.isOpening ? "Opening application" : "Searching")
                }
            }
            .padding(Theme.inset)
            Divider()
            content.frame(maxWidth: .infinity, maxHeight: .infinity)
            if let error = state.errorMessage {
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: "exclamationmark.triangle")
                    Text(error).font(Theme.subtitle).textSelection(.enabled)
                    Spacer(minLength: 0)
                    Button("Retry") {
                        if state.results.isEmpty { state.refresh() } else { open() }
                    }.disabled(state.isOpening || state.isSearching)
                }
                .padding(Theme.inset)
                .accessibilityElement(children: .contain)
            }
            Divider()
            HStack(spacing: 14) {
                Label("Fixtures", systemImage: "testtube.2")
                    .help("Development fixtures only. No Rust search or usage recording.")
                Spacer()
                Text("↑↓ Navigate")
                Text("↵ Open").foregroundStyle(state.canOpen ? Color.primary : Theme.secondary)
                Text("esc Close")
            }
            .font(Theme.hint).foregroundStyle(Theme.secondary)
            .padding(.horizontal, Theme.inset).padding(.vertical, 11)
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: Theme.corner))
        .overlay(RoundedRectangle(cornerRadius: Theme.corner).strokeBorder(.primary.opacity(0.12)))
    }

    @ViewBuilder private var content: some View {
        if state.results.isEmpty {
            VStack(spacing: 10) {
                Image(systemName: state.errorMessage != nil ? "exclamationmark.magnifyingglass" : "app.dashed")
                    .font(.system(size: 30)).foregroundStyle(Theme.secondary)
                Text(emptyTitle).font(.system(size: 16, weight: .medium))
                Text(state.query.isEmpty ? "Try Safari, Calendar, or IDLE. Type all to inspect every fixture." :
                        state.isSearching ? "" : state.errorMessage != nil ? "Change your query or retry below." : "Try another application name.")
                    .font(Theme.subtitle).foregroundStyle(Theme.secondary)
            }.padding(24)
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(state.results) { result in
                            row(result).id(result.id)
                        }
                    }.padding(8)
                }
                .onChange(of: state.selectedID) { _, id in
                    if let id { proxy.scrollTo(id, anchor: .center) }
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
        if state.query.isEmpty { return "Search fixture applications" }
        if state.isSearching { return state.showsProgress ? "Searching…" : "" }
        if state.errorMessage != nil { return "Search unavailable" }
        return "No matching applications"
    }

    private func row(_ result: AppResult) -> some View {
        let selected = state.selectedID == result.id
        return HStack(spacing: 12) {
            AppIcon(url: result.url)
            VStack(alignment: .leading, spacing: 3) {
                Text(result.name).font(Theme.title).lineLimit(1)
                Text(result.url.deletingLastPathComponent().path).font(Theme.subtitle)
                    .foregroundStyle(Theme.secondary).lineLimit(1).truncationMode(.middle)
            }
            Spacer(minLength: 8)
            if selected { Image(systemName: "return").font(Theme.subtitle).foregroundStyle(Theme.secondary) }
        }
        .padding(.horizontal, 12).frame(height: Theme.rowHeight)
        .background(selected ? Color.accentColor.opacity(0.13) : .clear, in: RoundedRectangle(cornerRadius: 7))
        .overlay(alignment: .leading) {
            if selected { RoundedRectangle(cornerRadius: 2).fill(Color.accentColor).frame(width: 3, height: 24) }
        }
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { state.select(result.id); open() }
        .onTapGesture { state.select(result.id) }
        .help(result.url.path)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(result.name), \(result.url.path)")
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : [.isButton])
        .accessibilityAction { state.select(result.id); open() }
        .accessibilityAction(named: "Select") { state.select(result.id) }
    }

    private func open() { Task { await state.openSelected() } }
}

private struct AppIcon: View {
    let url: URL
    @State private var image: NSImage?
    var body: some View {
        Group {
            if let image { Image(nsImage: image).resizable() }
            else { Image(systemName: "app.dashed").resizable().padding(5).foregroundStyle(Theme.secondary) }
        }
        .frame(width: 32, height: 32).accessibilityHidden(true)
        .task(id: url) {
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
        field.placeholderString = "Search applications…"
        field.setAccessibilityLabel("Search applications")
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
