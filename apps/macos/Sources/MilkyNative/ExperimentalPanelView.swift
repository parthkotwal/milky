import AppKit
import SwiftUI

public enum VisualConcept: String, CaseIterable, Sendable {
    case porcelain
    case milkGlass = "milk-glass"
    case opal

    public var title: String {
        switch self {
        case .porcelain: "Porcelain Index"
        case .milkGlass: "Milk Glass Desk"
        case .opal: "Opal Chamber"
        }
    }
}

private extension Color {
    init(rgb: UInt32) {
        self.init(red: Double((rgb >> 16) & 0xff) / 255,
                  green: Double((rgb >> 8) & 0xff) / 255,
                  blue: Double(rgb & 0xff) / 255)
    }
}

private struct StudyPalette {
    let canvas: Color
    let surface: Color
    let raised: Color
    let ink: Color
    let secondary: Color
    let accent: Color
    let rule: Color

    init(_ concept: VisualConcept, dark: Bool) {
        switch (concept, dark) {
        case (.porcelain, false):
            canvas = Color(rgb: 0xeee9df); surface = Color(rgb: 0xf8f5ef)
            raised = Color(rgb: 0xffffff); ink = Color(rgb: 0x252b2b)
            secondary = Color(rgb: 0x52605d); accent = Color(rgb: 0x805341)
            rule = Color(rgb: 0xc9c4b9)
        case (.porcelain, true):
            canvas = Color(rgb: 0x242728); surface = Color(rgb: 0x303333)
            raised = Color(rgb: 0x3d4140); ink = Color(rgb: 0xf8f2e9)
            secondary = Color(rgb: 0xc6cfca); accent = Color(rgb: 0xe5bca6)
            rule = Color(rgb: 0x626866)
        case (.milkGlass, false):
            canvas = Color(rgb: 0xe1edf0); surface = Color(rgb: 0xf2f8f8)
            raised = Color(rgb: 0xffffff); ink = Color(rgb: 0x19363d)
            secondary = Color(rgb: 0x4c6670); accent = Color(rgb: 0x1f6876)
            rule = Color(rgb: 0xb2ccd0)
        case (.milkGlass, true):
            canvas = Color(rgb: 0x162a33); surface = Color(rgb: 0x243c45)
            raised = Color(rgb: 0x31505a); ink = Color(rgb: 0xf4fbfa)
            secondary = Color(rgb: 0xb7d0d2); accent = Color(rgb: 0xa6e3df)
            rule = Color(rgb: 0x64818a)
        case (.opal, false):
            canvas = Color(rgb: 0xe4e8f1); surface = Color(rgb: 0xf8f8fc)
            raised = Color(rgb: 0xffffff); ink = Color(rgb: 0x20243b)
            secondary = Color(rgb: 0x4c526a); accent = Color(rgb: 0x5048a7)
            rule = Color(rgb: 0xb9bed8)
        case (.opal, true):
            canvas = Color(rgb: 0x111927); surface = Color(rgb: 0x222c40)
            raised = Color(rgb: 0x303b52); ink = Color(rgb: 0xf9f7ff)
            secondary = Color(rgb: 0xc0c8dc); accent = Color(rgb: 0xe2ccff)
            rule = Color(rgb: 0x65718a)
        }
    }
}

/// Experimental compositions share LauncherState and the native field editor.
/// Normal Rust launches continue to use LauncherView until a concept is chosen.
public struct ExperimentalPanelView: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Bindable var state: LauncherState
    let concept: VisualConcept
    let dismiss: () -> Void

    public init(state: LauncherState, concept: VisualConcept, dismiss: @escaping () -> Void) {
        self.state = state
        self.concept = concept
        self.dismiss = dismiss
    }

    private var palette: StudyPalette { StudyPalette(concept, dark: colorScheme == .dark) }
    private var activeResult: AppResult? { state.selected }

    public var body: some View {
        Group {
            switch concept {
            case .porcelain: porcelainComposition
            case .milkGlass: milkGlassComposition
            case .opal: opalComposition
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: concept == .opal ? 24 : 18))
        .overlay(RoundedRectangle(cornerRadius: concept == .opal ? 24 : 18)
            .strokeBorder(palette.rule.opacity(0.8), lineWidth: 1))
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.14), value: state.results)
        .onChange(of: state.selectedID) { _, _ in announceSelected() }
        .onChange(of: state.errorMessage) { _, message in
            if let message { announce(message) }
        }
        .onChange(of: state.isSearching) { _, searching in
            if !searching && state.results.isEmpty && state.errorMessage == nil && !state.query.isEmpty {
                announce("No matching results")
            }
        }
    }

    private func queryField() -> some View {
        QueryField(value: Binding(get: { state.query }, set: { state.setQuery($0) }),
                   enabled: !state.isOpening,
                   up: { state.moveSelection(-1) }, down: { state.moveSelection(1) },
                   submit: { Task { await state.openSelected() } },
                   dismiss: dismissOrClose,
                   acceptSuggestion: { state.acceptInlineSuggestion() })
            .frame(maxWidth: .infinity)
            .frame(height: 40)
            .accessibilityLabel("Search your Mac")
    }

    private func dismissOrClose() {
        if !state.closeOverlay() { dismiss() }
    }

    private func announceSelected() {
        guard let selected = state.selected,
              let index = state.results.firstIndex(where: { $0.id == selected.id }) else { return }
        announce("\(selected.title), \(selected.subtitle), \(selected.accessibilityDestination), result \(index + 1) of \(state.results.count)")
    }

    private func announce(_ message: String) {
        guard NSWorkspace.shared.isVoiceOverEnabled else { return }
        NSAccessibility.post(element: NSApp as Any, notification: .announcementRequested,
            userInfo: [.announcement: message, .priority: NSAccessibilityPriorityLevel.medium.rawValue])
    }

    private func resultControl(_ result: AppResult) -> some View {
        let selected = state.selectedID == result.id
        return HStack(spacing: 0) {
            ResultIcon(result: result)
            VStack(alignment: .leading, spacing: 3) {
                Text(result.title)
                    .font(.system(size: 14, weight: selected ? .semibold : .medium))
                    .foregroundStyle(palette.ink)
                    .lineLimit(1)
                Text(result.subtitle)
                    .font(.system(size: 11))
                    .foregroundStyle(palette.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            .padding(.leading, 10)
            Spacer(minLength: 6)
            Text(result.kind == .folder ? "FOLDER" : result.kind == .file ? "FILE" : result.kind.rawValue.uppercased())
                .font(.system(size: 9, weight: .semibold, design: .rounded))
                .tracking(0.5)
                .foregroundStyle(palette.secondary)
        }
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { state.select(result.id); Task { await state.openSelected() } }
        .onTapGesture { state.select(result.id) }
        .help(result.title + " — " + result.accessibilityDestination)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(result.title), \(result.subtitle), \(result.accessibilityDestination), \(result.kind.rawValue)")
        .accessibilityAddTraits(selected ? [.isSelected, .isButton] : [.isButton])
        .accessibilityAction { state.select(result.id); Task { await state.openSelected() } }
        .accessibilityAction(named: "Select") { state.select(result.id) }
    }

    private func actionLabel(_ action: ResultMenuAction, for result: AppResult) -> String {
        action.title(for: result)
    }

    private func actionSymbol(_ action: ResultMenuAction) -> String {
        switch action {
        case .open: "arrow.up.right.square"
        case .inspect: "eye"
        case .reveal: "folder"
        case .copyLocation: "doc.on.doc"
        }
    }
}

private extension ExperimentalPanelView {
    var porcelainComposition: some View {
        GeometryReader { geometry in
            HStack(spacing: 0) {
                porcelainRail(narrow: geometry.size.width < 520)
                    .frame(width: geometry.size.width < 520 ? 74 : 138)
                Rectangle().fill(palette.rule).frame(width: 1)
                VStack(spacing: 0) {
                    porcelainHeader
                    Rectangle().fill(palette.rule).frame(height: 1)
                    porcelainContent
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    Rectangle().fill(palette.rule).frame(height: 1)
                    porcelainFooter
                }
                .background(palette.surface)
            }
        }
        .background(palette.canvas)
    }

    func porcelainRail(narrow: Bool) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ZStack {
                RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(palette.accent, lineWidth: 1.5)
                    .frame(width: 45, height: 45)
                Text("M").font(.system(size: 29, weight: .light, design: .serif))
                    .foregroundStyle(palette.accent)
            }
            Text(narrow ? "" : "MILKY")
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .tracking(2.5)
                .foregroundStyle(palette.ink)
                .padding(.top, 15)
            if !narrow {
                Text("THE LOCAL INDEX")
                    .font(.system(size: 9, weight: .medium))
                    .tracking(1.1)
                    .foregroundStyle(palette.secondary)
                    .padding(.top, 5)
            }
            Rectangle().fill(palette.rule).frame(height: 1).padding(.vertical, 31)
            Text("01")
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .foregroundStyle(palette.accent)
            if !narrow {
                Text("FIND")
                    .font(.system(size: 10, weight: .semibold))
                    .tracking(1.2)
                    .foregroundStyle(palette.ink)
                    .padding(.top, 6)
            }
            Spacer()
            if !narrow {
                Text("PRIVATE\nBY DESIGN")
                    .font(.system(size: 9, weight: .medium))
                    .tracking(1.2)
                    .lineSpacing(4)
                    .foregroundStyle(palette.secondary)
            }
            Circle().fill(palette.accent).frame(width: 5, height: 5).padding(.top, 16)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(narrow ? 15 : 19)
        .background(palette.canvas)
    }

    var porcelainHeader: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("SEARCH / LOCAL DESTINATIONS")
                    .font(.system(size: 10, weight: .semibold, design: .rounded))
                    .tracking(1.35)
                    .foregroundStyle(palette.secondary)
                Spacer()
                Text("⌃⌥ SPACE")
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundStyle(palette.secondary)
            }
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 19, weight: .light))
                    .foregroundStyle(palette.accent)
                queryField()
            }
            HStack(spacing: 8) {
                if let suggestion = state.inlineSuggestion {
                    Button {
                        state.acceptInlineSuggestion()
                    } label: {
                        Text("TAB  →  \(suggestion.title)")
                            .lineLimit(1).truncationMode(.middle)
                    }
                    .buttonStyle(.plain)
                    .disabled(state.isSearching || state.isOpening)
                    .accessibilityLabel("Complete query with \(suggestion.title)")
                } else {
                    Text("NAMES, FILES, FOLDERS, ACTIONS")
                }
                Spacer()
                if state.showsProgress { ProgressView().controlSize(.small) }
            }
            .font(.system(size: 10, weight: .medium, design: .rounded))
            .tracking(0.6)
            .foregroundStyle(palette.secondary)
            .frame(height: 14)
        }
        .padding(.horizontal, 23)
        .padding(.vertical, 17)
    }

    @ViewBuilder var porcelainContent: some View {
        if let result = state.inspectedResult {
            porcelainInspector(result)
        } else if state.actionMenuResultID != nil, let result = activeResult {
            porcelainActions(result)
        } else if state.results.isEmpty {
            porcelainEmpty
        } else {
            VStack(spacing: 0) {
                HStack {
                    Text("DESTINATIONS")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .tracking(1.4)
                    Spacer()
                    Text(String(format: "%02d", state.results.count))
                        .font(.system(size: 11, weight: .medium, design: .monospaced))
                }
                .foregroundStyle(palette.secondary)
                .padding(.horizontal, 22)
                .padding(.top, 17)
                .padding(.bottom, 9)
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 5) {
                            ForEach(state.results) { result in
                                let selected = state.selectedID == result.id
                                HStack(spacing: 0) {
                                    Rectangle().fill(selected ? palette.accent : .clear)
                                        .frame(width: 3, height: 28)
                                    resultControl(result)
                                        .padding(.horizontal, 13)
                                }
                                .frame(height: 54)
                                .background(selected ? palette.raised : .clear,
                                            in: RoundedRectangle(cornerRadius: 8))
                                .overlay(RoundedRectangle(cornerRadius: 8)
                                    .strokeBorder(selected ? palette.rule : .clear, lineWidth: 1))
                                .id(result.id)
                            }
                        }
                        .padding(.horizontal, 12)
                        .padding(.bottom, 12)
                    }
                    .onChange(of: state.selectedID, initial: true) { _, id in
                        if let id { proxy.scrollTo(id, anchor: .center) }
                    }
                }
            }
        }
    }

    var porcelainEmpty: some View {
        VStack(alignment: .leading, spacing: 16) {
            Spacer()
            Text("FIND WHAT MATTERS")
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .tracking(1.7)
                .foregroundStyle(palette.accent)
            Text(state.errorMessage != nil ? "Search needs a moment." :
                 state.query.isEmpty ? "A place for every thing." :
                 state.isSearching ? "Looking through your Mac." : "Nothing found yet.")
                .font(.system(size: 30, weight: .light, design: .serif))
                .foregroundStyle(palette.ink)
                .fixedSize(horizontal: false, vertical: true)
            Text(state.query.isEmpty ? "Type a name, file, or folder to begin." :
                 state.errorMessage ?? "Try another name or path.")
                .font(.system(size: 13))
                .foregroundStyle(palette.secondary)
            Rectangle().fill(palette.accent).frame(width: 39, height: 2)
            Spacer()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 28)
    }

    func porcelainInspector(_ result: AppResult) -> some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 5) {
                    Text("FILE / INSPECTION")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .tracking(1.2).foregroundStyle(palette.accent)
                    Text(result.title).font(.system(size: 21, weight: .regular, design: .serif))
                        .foregroundStyle(palette.ink).lineLimit(1)
                }
                Spacer()
                Button("BACK") { state.closeOverlay() }
                    .font(.system(size: 10, weight: .semibold, design: .rounded))
                    .foregroundStyle(palette.accent)
                    .buttonStyle(.plain)
            }
            .padding(16)
            if case .open(let url) = result.action, FileManager.default.fileExists(atPath: url.path) {
                QuickLookFilePreview(url: url)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(palette.raised)
                    .clipShape(RoundedRectangle(cornerRadius: 7))
                    .padding(.horizontal, 13)
                    .padding(.bottom, 13)
                    .accessibilityLabel("Preview of \(result.title)")
            } else {
                Text("This file is no longer available.")
                    .foregroundStyle(palette.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }

    func porcelainActions(_ result: AppResult) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("ACTIONS / \(result.kind.rawValue.uppercased())")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .tracking(1.2).foregroundStyle(palette.accent)
                    Text(result.title).font(.system(size: 22, weight: .regular, design: .serif))
                        .foregroundStyle(palette.ink).lineLimit(1)
                }
                Spacer()
                Button("BACK") { state.closeOverlay() }
                    .font(.system(size: 10, weight: .semibold, design: .rounded))
                    .foregroundStyle(palette.accent).buttonStyle(.plain)
            }
            .padding(20)
            Rectangle().fill(palette.rule).frame(height: 1)
            ForEach(Array(state.menuActions.enumerated()), id: \.element) { index, action in
                let highlighted = state.selectedActionIndex == index
                Button { Task { await state.chooseMenuAction(action) } } label: {
                    HStack(spacing: 14) {
                        Text(String(format: "%02d", index + 1))
                            .font(.system(size: 11, weight: .medium, design: .monospaced))
                            .foregroundStyle(palette.accent)
                        Image(systemName: actionSymbol(action)).frame(width: 22)
                        Text(actionLabel(action, for: result))
                        Spacer()
                        if highlighted { Image(systemName: "return") }
                    }
                    .font(.system(size: 14, weight: highlighted ? .semibold : .regular))
                    .foregroundStyle(palette.ink)
                    .padding(.horizontal, 20)
                    .frame(height: 53)
                    .background(highlighted ? palette.raised : .clear)
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(highlighted ? .isSelected : [])
                Rectangle().fill(palette.rule).frame(height: 1).padding(.leading, 20)
            }
            Spacer(minLength: 0)
        }
    }

    var porcelainFooter: some View {
        HStack(spacing: 10) {
            Text("DESIGN STUDY")
                .tracking(1.1)
            Spacer(minLength: 0)
            if state.actionMenuResultID != nil {
                Text("↑↓ CHOOSE  ↵ RUN  ESC BACK")
            } else if state.inspectedResultID != nil {
                Text("ESC BACK  ↵ OPEN")
            } else {
                if activeResult?.kind == .file {
                    Button("⌘P INSPECT") { state.toggleInspection() }.buttonStyle(.plain)
                }
                if activeResult != nil {
                    Button("⌘K ACTIONS") { state.toggleActionMenu() }.buttonStyle(.plain)
                }
                Text("↵ OPEN")
            }
        }
        .font(.system(size: 9, weight: .semibold, design: .rounded))
        .foregroundStyle(palette.secondary)
        .padding(.horizontal, 19)
        .frame(height: 37)
    }
}

private extension ExperimentalPanelView {
    var opalComposition: some View {
        ZStack {
            palette.canvas
            if !reduceTransparency {
                RadialGradient(colors: [Color(rgb: 0x7774bb).opacity(colorScheme == .dark ? 0.30 : 0.22),
                                        .clear], center: .topTrailing, startRadius: 10, endRadius: 370)
                RadialGradient(colors: [Color(rgb: 0x62c6c3).opacity(colorScheme == .dark ? 0.22 : 0.28),
                                        .clear], center: .bottomLeading, startRadius: 20, endRadius: 360)
            }
            VStack(spacing: 0) {
                HStack(spacing: 11) {
                    ZStack {
                        Circle()
                            .fill(AngularGradient(colors: [Color(rgb: 0x8acbd0),
                                                          Color(rgb: 0xd3bded),
                                                          Color(rgb: 0xf3e5ca),
                                                          Color(rgb: 0x8acbd0)], center: .center))
                            .frame(width: 31, height: 31)
                        Circle().fill(palette.canvas).frame(width: 13, height: 13)
                    }
                    Text("MILKY")
                        .font(.system(size: 14, weight: .bold, design: .rounded))
                        .tracking(4)
                        .foregroundStyle(palette.ink)
                    Spacer()
                    Text("LOCAL / 001")
                        .font(.system(size: 10, weight: .semibold, design: .monospaced))
                        .foregroundStyle(palette.secondary)
                }
                .padding(.horizontal, 25)
                .padding(.top, 18)
                HStack(spacing: 12) {
                    Image(systemName: "sparkle.magnifyingglass")
                        .font(.system(size: 24, weight: .ultraLight))
                        .foregroundStyle(palette.accent)
                    queryField()
                    if state.showsProgress { ProgressView().controlSize(.small) }
                    Text("↵")
                        .font(.system(size: 15, weight: .medium, design: .monospaced))
                        .foregroundStyle(palette.secondary)
                }
                .padding(.horizontal, 18)
                .frame(height: 65)
                .background(palette.raised, in: RoundedRectangle(cornerRadius: 20))
                .overlay(RoundedRectangle(cornerRadius: 20)
                    .strokeBorder(palette.accent.opacity(0.58), lineWidth: 1.4))
                .shadow(color: palette.accent.opacity(0.13), radius: 16, y: 5)
                .padding(.horizontal, 23)
                .padding(.top, 22)
                HStack {
                    if let suggestion = state.inlineSuggestion {
                        Button { state.acceptInlineSuggestion() } label: {
                            HStack(spacing: 7) {
                                Text("TAB")
                                    .font(.system(size: 9, weight: .bold, design: .monospaced))
                                    .padding(.horizontal, 7).padding(.vertical, 4)
                                    .background(palette.raised, in: Capsule())
                                Text("\(suggestion.title)")
                                    .lineLimit(1).truncationMode(.middle)
                            }
                        }
                        .buttonStyle(.plain)
                        .disabled(state.isSearching || state.isOpening)
                        .accessibilityLabel("Complete query with \(suggestion.title)")
                    } else {
                        Text("FIND A THING. DO A THING.")
                    }
                    Spacer()
                    Text("DESIGN STUDY")
                }
                .font(.system(size: 10, weight: .semibold, design: .rounded))
                .tracking(1)
                .foregroundStyle(palette.secondary)
                .padding(.horizontal, 31)
                .frame(height: 39)
                opalContent
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .padding(.horizontal, 22)
                opalFooter
            }
        }
    }

    @ViewBuilder var opalContent: some View {
        if let result = state.inspectedResult {
            opalInspector(result)
        } else if state.actionMenuResultID != nil, let result = activeResult {
            opalActions(result)
        } else if state.results.isEmpty {
            opalEmpty
        } else {
            VStack(spacing: 8) {
                HStack {
                    Text("RESULTS IN ORBIT")
                        .font(.system(size: 10, weight: .bold, design: .rounded))
                        .tracking(1.6)
                    Spacer()
                    Text(String(format: "%02d", state.results.count))
                        .font(.system(size: 11, weight: .semibold, design: .monospaced))
                }
                .foregroundStyle(palette.secondary)
                .padding(.horizontal, 4)
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 6) {
                            ForEach(Array(state.results.enumerated()), id: \.element.id) { index, result in
                                let selected = state.selectedID == result.id
                                HStack(spacing: 10) {
                                    Text(String(format: "%02d", index + 1))
                                        .font(.system(size: 11, weight: .semibold, design: .monospaced))
                                        .foregroundStyle(palette.accent)
                                        .frame(width: 28)
                                    Rectangle().fill(selected ? palette.accent : palette.rule)
                                        .frame(width: 1, height: 27)
                                    resultControl(result)
                                }
                                .padding(.horizontal, 12)
                                .frame(height: selected ? 58 : 49)
                                .background(selected ? palette.raised : palette.surface,
                                            in: RoundedRectangle(cornerRadius: 13))
                                .overlay(RoundedRectangle(cornerRadius: 13)
                                    .strokeBorder(selected ? palette.accent : palette.rule.opacity(0.55),
                                                  lineWidth: selected ? 1.4 : 0.8))
                                .id(result.id)
                            }
                        }
                        .padding(.vertical, 3)
                    }
                    .onChange(of: state.selectedID, initial: true) { _, id in
                        if let id { proxy.scrollTo(id, anchor: .center) }
                    }
                }
            }
        }
    }

    var opalEmpty: some View {
        ZStack {
            Circle()
                .strokeBorder(AngularGradient(colors: [Color(rgb: 0x8acbd0),
                                                      Color(rgb: 0xd3bded),
                                                      Color(rgb: 0xf3e5ca),
                                                      Color(rgb: 0x8acbd0)], center: .center), lineWidth: 15)
                .frame(width: 210, height: 210)
                .blur(radius: reduceTransparency ? 0 : 0.5)
                .opacity(colorScheme == .dark ? 0.58 : 0.68)
            Circle().fill(palette.canvas).frame(width: 172, height: 172)
            VStack(spacing: 9) {
                Text(state.errorMessage != nil ? "TRY AGAIN" :
                     state.query.isEmpty ? "ANYTHING,\nANYWHERE." :
                     state.isSearching ? "SEARCHING." : "NO MATCH.")
                    .font(.system(size: 22, weight: .bold, design: .rounded))
                    .tracking(1.2)
                    .multilineTextAlignment(.center)
                    .foregroundStyle(palette.ink)
                Text(state.query.isEmpty ? "LOCAL SEARCH IS READY" :
                     state.errorMessage ?? "TRY ANOTHER NAME")
                    .font(.system(size: 9, weight: .semibold, design: .rounded))
                    .tracking(1)
                    .foregroundStyle(palette.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    func opalInspector(_ result: AppResult) -> some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("04 / FILE INSPECTION")
                        .font(.system(size: 10, weight: .bold, design: .rounded))
                        .tracking(1.5).foregroundStyle(palette.accent)
                    Text(result.title)
                        .font(.system(size: 21, weight: .semibold, design: .rounded))
                        .foregroundStyle(palette.ink).lineLimit(1)
                }
                Spacer()
                Button("BACK  ESC") { state.closeOverlay() }
                    .font(.system(size: 10, weight: .bold, design: .rounded))
                    .foregroundStyle(palette.accent).buttonStyle(.plain)
            }
            .padding(15)
            if case .open(let url) = result.action, FileManager.default.fileExists(atPath: url.path) {
                QuickLookFilePreview(url: url)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(palette.raised)
                    .clipShape(RoundedRectangle(cornerRadius: 11))
                    .padding(.horizontal, 10)
                    .padding(.bottom, 10)
                    .accessibilityLabel("Preview of \(result.title)")
            } else {
                Text("This file is no longer available.")
                    .foregroundStyle(palette.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(palette.surface, in: RoundedRectangle(cornerRadius: 17))
        .overlay(RoundedRectangle(cornerRadius: 17)
            .strokeBorder(palette.accent.opacity(0.7), lineWidth: 1))
    }

    func opalActions(_ result: AppResult) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("ACTION CONSTELLATION")
                        .font(.system(size: 10, weight: .bold, design: .rounded))
                        .tracking(1.5).foregroundStyle(palette.accent)
                    Text(result.title)
                        .font(.system(size: 22, weight: .semibold, design: .rounded))
                        .foregroundStyle(palette.ink).lineLimit(1)
                }
                Spacer()
                Button("BACK  ESC") { state.closeOverlay() }
                    .font(.system(size: 10, weight: .bold, design: .rounded))
                    .foregroundStyle(palette.accent).buttonStyle(.plain)
            }
            .padding(.bottom, 7)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(spacing: 7) {
                        ForEach(Array(state.menuActions.enumerated()), id: \.element) { index, action in
                            let highlighted = state.selectedActionIndex == index
                            Button { Task { await state.chooseMenuAction(action) } } label: {
                                HStack(spacing: 15) {
                                    Text(String(format: "%02d", index + 1))
                                        .font(.system(size: 11, weight: .bold, design: .monospaced))
                                        .foregroundStyle(palette.accent)
                                    Image(systemName: actionSymbol(action)).frame(width: 20)
                                    Text(actionLabel(action, for: result))
                                    Spacer()
                                    if highlighted { Image(systemName: "return") }
                                }
                                .font(.system(size: 14, weight: highlighted ? .bold : .medium, design: .rounded))
                                .foregroundStyle(palette.ink)
                                .padding(.horizontal, 15)
                                .frame(height: 50)
                                .background(highlighted ? palette.raised : palette.surface,
                                            in: RoundedRectangle(cornerRadius: 12))
                                .overlay(RoundedRectangle(cornerRadius: 12)
                                    .strokeBorder(highlighted ? palette.accent : palette.rule.opacity(0.6),
                                                  lineWidth: highlighted ? 1.2 : 0.7))
                            }
                            .buttonStyle(.plain)
                            .accessibilityAddTraits(highlighted ? .isSelected : [])
                            .id(index)
                        }
                    }
                }
                .onChange(of: state.selectedActionIndex, initial: true) { _, index in
                    proxy.scrollTo(index, anchor: .center)
                }
            }
        }
        .padding(15)
        .background(palette.surface, in: RoundedRectangle(cornerRadius: 17))
        .overlay(RoundedRectangle(cornerRadius: 17)
            .strokeBorder(palette.accent.opacity(0.7), lineWidth: 1))
    }

    var opalFooter: some View {
        HStack(spacing: 12) {
            Text("✦  FULLY LOCAL")
                .foregroundStyle(palette.accent)
            Spacer(minLength: 0)
            if state.actionMenuResultID != nil {
                Text("↑↓ CHOOSE    ↵ RUN    ESC BACK")
            } else if state.inspectedResultID != nil {
                Text("ESC BACK    ↵ OPEN")
            } else {
                if activeResult?.kind == .file {
                    Button("⌘P INSPECT") { state.toggleInspection() }.buttonStyle(.plain)
                }
                if activeResult != nil {
                    Button("⌘K ACTIONS") { state.toggleActionMenu() }.buttonStyle(.plain)
                }
                Text("↵ OPEN")
            }
        }
        .font(.system(size: 9, weight: .bold, design: .rounded))
        .tracking(0.8)
        .foregroundStyle(palette.secondary)
        .padding(.horizontal, 26)
        .frame(height: 44)
    }
}

private extension ExperimentalPanelView {
    var milkGlassComposition: some View {
        ZStack {
            LinearGradient(colors: [palette.canvas,
                                    colorScheme == .dark ? Color(rgb: 0x2a334a) : Color(rgb: 0xeee5f0)],
                           startPoint: .topLeading, endPoint: .bottomTrailing)
            if !reduceTransparency {
                Circle()
                    .fill(Color(rgb: 0xb2dfdf).opacity(colorScheme == .dark ? 0.16 : 0.6))
                    .frame(width: 310, height: 310)
                    .blur(radius: 55)
                    .offset(x: 260, y: -220)
                Circle()
                    .fill(Color(rgb: 0xdbcaed).opacity(colorScheme == .dark ? 0.12 : 0.5))
                    .frame(width: 240, height: 240)
                    .blur(radius: 50)
                    .offset(x: -260, y: 200)
            }
            VStack(spacing: 0) {
                HStack {
                    HStack(spacing: 7) {
                        Circle().fill(palette.accent).frame(width: 10, height: 10)
                        Circle().fill(Color(rgb: 0xbcc4e8)).frame(width: 6, height: 6)
                        Text("milky")
                            .font(.system(size: 19, weight: .semibold, design: .rounded))
                            .tracking(-0.7)
                    }
                    .foregroundStyle(palette.ink)
                    Spacer()
                    Text("LOCAL SEARCH  /  DESIGN STUDY")
                        .font(.system(size: 9, weight: .semibold, design: .rounded))
                        .tracking(1)
                        .foregroundStyle(palette.secondary)
                }
                .padding(.horizontal, 20)
                .padding(.top, 16)
                .padding(.bottom, 12)
                HStack(spacing: 11) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 18, weight: .medium))
                        .foregroundStyle(palette.accent)
                    queryField()
                    if state.showsProgress { ProgressView().controlSize(.small) }
                }
                .padding(.horizontal, 16)
                .frame(height: 56)
                .background(palette.raised, in: RoundedRectangle(cornerRadius: 15))
                .overlay(RoundedRectangle(cornerRadius: 15)
                    .strokeBorder(palette.rule.opacity(0.7), lineWidth: 1))
                .padding(.horizontal, 18)
                .padding(.bottom, 14)
                GeometryReader { geometry in
                    if geometry.size.width < 520 {
                        if state.inspectedResult != nil || state.actionMenuResultID != nil || state.results.isEmpty {
                            milkFocusPane
                        } else {
                            milkResultsPane
                        }
                    } else {
                        HStack(spacing: 11) {
                            milkResultsPane.frame(width: geometry.size.width * 0.52)
                            milkFocusPane.frame(maxWidth: .infinity)
                        }
                    }
                }
                .padding(.horizontal, 18)
                milkFooter
            }
        }
    }

    var milkResultsPane: some View {
        VStack(spacing: 0) {
            HStack {
                Text("RANKED RESULTS")
                    .tracking(1.1)
                Spacer()
                Text(state.results.isEmpty ? "—" : "\(state.results.count)")
            }
            .font(.system(size: 10, weight: .semibold, design: .rounded))
            .foregroundStyle(palette.secondary)
            .padding(.horizontal, 15)
            .frame(height: 37)
            Rectangle().fill(palette.rule.opacity(0.7)).frame(height: 1)
            if state.results.isEmpty {
                VStack(alignment: .leading, spacing: 11) {
                    Spacer()
                    Text("The right thing,\nright here.")
                        .font(.system(size: 26, weight: .light, design: .rounded))
                        .foregroundStyle(palette.ink)
                    Text(state.errorMessage ?? (state.query.isEmpty ?
                        "Type a file, folder, or app name." :
                        state.isSearching ? "Searching locally…" : "No matches. Try another name."))
                        .font(.system(size: 12))
                        .foregroundStyle(palette.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(20)
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 3) {
                            ForEach(state.results) { result in
                                let selected = state.selectedID == result.id
                                resultControl(result)
                                    .padding(.horizontal, 10)
                                    .frame(height: 48)
                                    .background(selected ? palette.raised : .clear,
                                                in: RoundedRectangle(cornerRadius: 10))
                                    .overlay(RoundedRectangle(cornerRadius: 10)
                                        .strokeBorder(selected ? palette.accent.opacity(0.55) : .clear, lineWidth: 1))
                                    .id(result.id)
                            }
                        }
                        .padding(7)
                    }
                    .onChange(of: state.selectedID, initial: true) { _, id in
                        if let id { proxy.scrollTo(id, anchor: .center) }
                    }
                }
            }
        }
        .background(palette.surface, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16)
            .strokeBorder(palette.rule.opacity(0.6), lineWidth: 1))
    }

    @ViewBuilder var milkFocusPane: some View {
        if let result = state.inspectedResult {
            milkInspector(result)
        } else if state.actionMenuResultID != nil, let result = activeResult {
            milkActions(result)
        } else if let result = activeResult, !state.isSearching {
            milkSelected(result)
        } else {
            ZStack {
                RoundedRectangle(cornerRadius: 16).fill(palette.surface)
                VStack(spacing: 17) {
                    ZStack {
                        Circle().fill(palette.canvas).frame(width: 112, height: 112)
                        Circle().strokeBorder(palette.rule, lineWidth: 1)
                            .frame(width: 112, height: 112)
                        Image(systemName: "sparkle.magnifyingglass")
                            .font(.system(size: 35, weight: .ultraLight))
                            .foregroundStyle(palette.accent)
                    }
                    Text("Your Mac, in focus")
                        .font(.system(size: 20, weight: .medium, design: .rounded))
                        .foregroundStyle(palette.ink)
                    Text("Search stays here.\nThe result comes into view.")
                        .font(.system(size: 12))
                        .multilineTextAlignment(.center)
                        .foregroundStyle(palette.secondary)
                }
            }
            .overlay(RoundedRectangle(cornerRadius: 16)
                .strokeBorder(palette.rule.opacity(0.6), lineWidth: 1))
        }
    }

    func milkSelected(_ result: AppResult) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text("SELECTED DESTINATION")
                    .font(.system(size: 10, weight: .semibold, design: .rounded))
                    .tracking(1)
                    .foregroundStyle(palette.secondary)
                Spacer()
                Image(systemName: "arrow.up.right")
                    .foregroundStyle(palette.accent)
            }
            ResultIcon(result: result)
                .scaleEffect(1.55)
                .frame(width: 58, height: 58)
                .padding(.top, 31)
            Text(result.title)
                .font(.system(size: 24, weight: .medium, design: .rounded))
                .foregroundStyle(palette.ink)
                .lineLimit(2)
                .padding(.top, 18)
            Text(result.subtitle)
                .font(.system(size: 12))
                .foregroundStyle(palette.secondary)
                .lineLimit(2).truncationMode(.middle)
                .padding(.top, 7)
            Spacer(minLength: 5)
            if let suggestion = state.inlineSuggestion {
                Button { state.acceptInlineSuggestion() } label: {
                    HStack {
                        Text("TAB TO COMPLETE")
                            .font(.system(size: 10, weight: .semibold, design: .rounded))
                            .tracking(0.8)
                        Spacer()
                        Text(suggestion.title).lineLimit(1)
                    }
                }
                .buttonStyle(.plain)
                .disabled(state.isSearching || state.isOpening)
                .foregroundStyle(palette.accent)
                .padding(.bottom, 12)
            }
            Rectangle().fill(palette.rule).frame(height: 1)
            HStack {
                if result.kind == .file {
                    Button("Inspect  ⌘P") { state.toggleInspection() }
                        .buttonStyle(.plain)
                }
                Spacer()
                Button("Actions  ⌘K") { state.toggleActionMenu() }
                    .buttonStyle(.plain)
            }
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(palette.accent)
            .padding(.top, 13)
        }
        .padding(18)
        .background(palette.surface, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16)
            .strokeBorder(palette.rule.opacity(0.6), lineWidth: 1))
    }

    func milkInspector(_ result: AppResult) -> some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("IN FOCUS")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .tracking(1).foregroundStyle(palette.accent)
                    Text(result.title).font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(palette.ink).lineLimit(1)
                }
                Spacer()
                Button("Back") { state.closeOverlay() }
                    .buttonStyle(.plain).foregroundStyle(palette.accent)
            }
            .padding(13)
            Rectangle().fill(palette.rule).frame(height: 1)
            if case .open(let url) = result.action, FileManager.default.fileExists(atPath: url.path) {
                QuickLookFilePreview(url: url)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityLabel("Preview of \(result.title)")
            } else {
                Text("This file is no longer available.")
                    .foregroundStyle(palette.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(palette.raised, in: RoundedRectangle(cornerRadius: 16))
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16)
            .strokeBorder(palette.rule.opacity(0.6), lineWidth: 1))
    }

    func milkActions(_ result: AppResult) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("FOR \(result.kind.rawValue.uppercased())")
                        .font(.system(size: 10, weight: .semibold, design: .rounded))
                        .tracking(1).foregroundStyle(palette.accent)
                    Text(result.title).font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(palette.ink).lineLimit(1)
                }
                Spacer()
                Button("Back") { state.closeOverlay() }
                    .buttonStyle(.plain).foregroundStyle(palette.accent)
            }
            .padding(14)
            Rectangle().fill(palette.rule).frame(height: 1)
            ForEach(Array(state.menuActions.enumerated()), id: \.element) { index, action in
                let highlighted = state.selectedActionIndex == index
                Button { Task { await state.chooseMenuAction(action) } } label: {
                    HStack(spacing: 9) {
                        Image(systemName: actionSymbol(action)).frame(width: 19)
                        Text(actionLabel(action, for: result))
                        Spacer(minLength: 1)
                        if highlighted { Image(systemName: "return") }
                    }
                    .font(.system(size: 12, weight: highlighted ? .semibold : .medium))
                    .foregroundStyle(palette.ink)
                    .padding(.horizontal, 10)
                    .frame(height: 47)
                    .background(highlighted ? palette.raised : .clear,
                                in: RoundedRectangle(cornerRadius: 8))
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(highlighted ? .isSelected : [])
                .padding(.horizontal, 8)
                .padding(.top, 5)
            }
            Spacer(minLength: 0)
        }
        .background(palette.surface, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16)
            .strokeBorder(palette.rule.opacity(0.6), lineWidth: 1))
    }

    var milkFooter: some View {
        HStack {
            Text("FIXTURE  ·  PRIVATE BY DESIGN")
            Spacer()
            Text(state.actionMenuResultID != nil ? "↑↓ Choose   ↵ Run   Esc Back" :
                 state.inspectedResultID != nil ? "Esc Back   ↵ Open" :
                 "↑↓ Navigate   ↵ Open   Esc Close")
        }
        .font(.system(size: 10, weight: .medium, design: .rounded))
        .foregroundStyle(palette.secondary)
        .padding(.horizontal, 20)
        .frame(height: 38)
    }
}
