import AppKit
import Carbon
import MilkyNative
import MilkyRust
import SwiftUI

@main
struct MilkyLauncher {
    @MainActor static func main() {
        let app = NSApplication.shared
        let delegate = LauncherDelegate()
        app.delegate = delegate
        withExtendedLifetime(delegate) { app.run() }
    }
}

private final class LauncherPanel: NSPanel {
    var dismissAction: (() -> Void)?
    override func cancelOperation(_ sender: Any?) { dismissAction?() }
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

@MainActor
private final class LauncherDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private let fixtures = CommandLine.arguments.contains("--fixtures")
    private var panelDesign: PanelDesign {
        guard fixtures,
              let value = CommandLine.arguments.first(where: { $0.hasPrefix("--design=") })?.dropFirst("--design=".count),
              let design = PanelDesign(rawValue: String(value)) else { return .hybrid }
        return design
    }
    private let rust = RustSearchProvider()
    private lazy var state = LauncherState(provider: fixtures ? FixtureSearchProvider() : rust,
                                           opener: NativeAppOpener(),
                                           eventRecorder: fixtures ? nil : rust)
    private var preparation: Task<Void, Never>?
    private var terminating = false
    private var displayName: String { fixtures ? "Milky — Fixtures" : "Milky" }
    private var panel: LauncherPanel!
    private var statusItem: NSStatusItem!
    private var shortcut: GlobalShortcut?
    private var session = InvocationSession()
    private var lastExternalPID: Int32?
    private let diagnostics = LifecycleDiagnostics()
    private var workspaceObserver: NSObjectProtocol?
    private var screenObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installMainMenu()
        let front = NSWorkspace.shared.frontmostApplication
        if front?.processIdentifier != ProcessInfo.processInfo.processIdentifier { lastExternalPID = front?.processIdentifier }
        workspaceObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didActivateApplicationNotification, object: nil, queue: .main
        ) { [weak self] notification in
            let pid = (notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication)?.processIdentifier
            MainActor.assumeIsolated {
                guard let self, let pid, pid != ProcessInfo.processInfo.processIdentifier,
                      NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { return }
                self.lastExternalPID = pid
                self.record("external-activation")
                if self.session.isPresented && !self.state.isOpening { self.dismiss(restoreFocus: false) }
            }
        }
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.repositionForDisplayChange() }
        }
        panel = LauncherPanel(contentRect: NSRect(x: 0, y: 0, width: 640, height: 554),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        panel.title = displayName
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        panel.dismissAction = { [weak self] in self?.dismissOrClose() }
        panel.contentView = NSHostingView(rootView: LauncherView(state: state, fixtures: fixtures,
                                                                 design: panelDesign) { [weak self] in
            self?.dismissOrClose()
        })
        if CommandLine.arguments.contains("--appearance=light") { panel.appearance = NSAppearance(named: .aqua) }
        if CommandLine.arguments.contains("--appearance=dark") { panel.appearance = NSAppearance(named: .darkAqua) }
        state.onOpened = { [weak self] in
            self?.record("action-opened")
            self?.dismiss(restoreFocus: false)
        }

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = NSImage(systemSymbolName: "magnifyingglass.circle", accessibilityDescription: displayName)
        let menu = NSMenu()
        let showItem = menu.addItem(withTitle: "Show \(displayName)", action: #selector(showFromMenu), keyEquivalent: "")
        showItem.target = self
        menu.addItem(.separator())
        let quit = menu.addItem(withTitle: "Quit Milky", action: #selector(quitApp), keyEquivalent: "q")
        quit.target = self
        statusItem.menu = menu

        let shortcutOption = CommandLine.arguments.first { $0.hasPrefix("--shortcut=") }
            .map { String($0.dropFirst("--shortcut=".count)) }
        let modifiers: Int
        let label: String
        switch shortcutOption {
        case "command-space":
            modifiers = cmdKey
            label = "⌘Space"
        case "command-shift-space":
            modifiers = cmdKey | shiftKey
            label = "⇧⌘Space"
        default:
            modifiers = controlKey | optionKey
            label = "⌃⌥Space"
        }
        shortcut = GlobalShortcut(modifiers: UInt32(modifiers)) { [weak self] in
            guard let self else { return }
            self.record("hotkey")
            if self.session.isPresented { self.dismiss(restoreFocus: true) } else { self.show() }
        }
        if shortcut == nil {
            let item = NSMenuItem(title: "Shortcut unavailable — use Show Milky", action: nil, keyEquivalent: "")
            menu.insertItem(item, at: 1)
            fputs("Milky: global shortcut registration failed; use the menu bar item.\n", stderr)
        } else {
            showItem.title = "Show \(displayName)  \(label)"
        }
        record(shortcut == nil ? "shortcut-unavailable" : "shortcut-registered")
        show()
        if !fixtures {
            preparation = Task { [weak self, rust] in
                do { try await rust.prepare() }
                catch { self?.state.showProviderError(error) }
            }
        }
    }

    private func installMainMenu() {
        let main = NSMenu()
        let appItem = NSMenuItem()
        let appMenu = NSMenu(title: "Milky")
        appMenu.addItem(withTitle: "Quit Milky", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)
        let editItem = NSMenuItem()
        let edit = NSMenu(title: "Edit")
        edit.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = edit.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        edit.addItem(.separator())
        edit.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        edit.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        edit.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        edit.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = edit
        main.addItem(editItem)
        let actionsItem = NSMenuItem()
        let actionsMenu = NSMenu(title: "Results")
        let inspect = actionsMenu.addItem(withTitle: "Inspect File", action: #selector(inspectSelectedFile), keyEquivalent: "p")
        inspect.target = self
        let showActions = actionsMenu.addItem(withTitle: "Show Actions", action: #selector(showSelectedActions), keyEquivalent: "k")
        showActions.target = self
        actionsItem.submenu = actionsMenu
        main.addItem(actionsItem)
        NSApp.mainMenu = main
    }

    @objc private func showFromMenu() { show() }
    @objc private func quitApp() { NSApp.terminate(nil) }
    @objc private func inspectSelectedFile() {
        guard session.isPresented else { return }
        state.toggleInspection()
    }
    @objc private func showSelectedActions() {
        guard session.isPresented else { return }
        state.toggleActionMenu()
    }

    private func dismissOrClose() {
        if !state.closeOverlay() { dismiss(restoreFocus: true) }
    }

    private func show() {
        guard !terminating else { return }
        if !session.isPresented {
            let front = NSWorkspace.shared.frontmostApplication
            let previous = front?.processIdentifier == ProcessInfo.processInfo.processIdentifier ? lastExternalPID : front?.processIdentifier
            session.begin(previousPID: previous)
            state.beginSession()
            repositionForDisplayChange()
        }
        record("show")
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
        panel.contentView?.layoutSubtreeIfNeeded()
        focusSearch()
        // SwiftUI may attach its NSTextField after the initial hosting layout.
        let token = session.generation
        DispatchQueue.main.async { [weak self] in
            guard let self, self.session.generation == token else { return }
            self.focusSearch()
        }
    }

    private func focusSearch() {
        guard session.isPresented, panel.isVisible, panel.isKeyWindow else { return }
        if let field = findField(panel.contentView), field.currentEditor() == nil {
            panel.makeFirstResponder(field)
        }
        record("search-focused")
    }

    private func findField(_ view: NSView?) -> NSTextField? {
        if let field = view as? NSTextField { return field }
        for child in view?.subviews ?? [] {
            if let field = findField(child) { return field }
        }
        return nil
    }

    private func dismiss(restoreFocus: Bool) {
        guard session.isPresented else { return }
        let ownsFocus = NSWorkspace.shared.frontmostApplication?.processIdentifier == ProcessInfo.processInfo.processIdentifier
        let target = session.dismiss(restoreFocus: restoreFocus, ownsFocus: ownsFocus)
        record(restoreFocus ? "dismiss-return" : "dismiss-no-return")
        state.dismiss()
        panel.orderOut(nil)
        if let target, let app = NSRunningApplication(processIdentifier: target), !app.isTerminated {
            let accepted = app.activate(options: [])
            record(accepted ? "return-accepted" : "return-rejected")
        }
    }

    private func repositionForDisplayChange() {
        guard session.isPresented, let panel else { return }
        let mouse = NSEvent.mouseLocation
        let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) } ?? NSScreen.main
        if let screen { panel.setFrame(PanelGeometry.frame(in: screen.visibleFrame), display: true) }
    }

    private func record(_ event: String) {
        diagnostics.record(event, panel: panel,
            session: InvocationSessionSnapshot(presented: session.isPresented, generation: session.generation))
    }

    func windowDidBecomeKey(_ notification: Notification) {
        let token = session.generation
        DispatchQueue.main.async { [weak self] in
            guard let self, self.session.generation == token else { return }
            self.focusSearch()
        }
    }

    func windowDidResignKey(_ notification: Notification) {
        // Resigning key alone is not enough evidence that the user left Milky:
        // mouse and accessibility activation can transiently cause it. The
        // workspace activation observer above dismisses only for a real new
        // foreground application.
        record("resign-key")
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        record("reopen")
        show()
        return true
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard !terminating else { return .terminateLater }
        terminating = true
        state.dismiss()
        preparation?.cancel()
        shortcut?.invalidate()
        Task {
            await rust.shutdown()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }

    func applicationWillTerminate(_ notification: Notification) {
        state.dismiss()
        shortcut?.invalidate()
        NSStatusBar.system.removeStatusItem(statusItem)
        if let workspaceObserver { NSWorkspace.shared.notificationCenter.removeObserver(workspaceObserver) }
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
        record("terminated")
        diagnostics.close()
    }
}

@MainActor
private final class GlobalShortcut {
    private var hotKey: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private let action: () -> Void
    private var pressGate = ShortcutPressGate()

    init?(modifiers: UInt32, action: @escaping () -> Void) {
        self.action = action
        var eventTypes = [
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased)),
        ]
        let context = Unmanaged.passUnretained(self).toOpaque()
        let installed = InstallEventHandler(GetApplicationEventTarget(), { _, event, context in
            guard let context, let event else { return OSStatus(eventNotHandledErr) }
            var id = EventHotKeyID()
            guard GetEventParameter(event, EventParamName(kEventParamDirectObject), EventParamType(typeEventHotKeyID),
                                    nil, MemoryLayout<EventHotKeyID>.size, nil, &id) == noErr,
                  id.signature == 0x4D494C4B, id.id == 1 else { return OSStatus(eventNotHandledErr) }
            MainActor.assumeIsolated {
                let shortcut = Unmanaged<GlobalShortcut>.fromOpaque(context).takeUnretainedValue()
                if GetEventKind(event) == UInt32(kEventHotKeyReleased) { shortcut.pressGate.keyUp() }
                else if shortcut.pressGate.keyDown() { shortcut.action() }
            }
            return noErr
        }, 2, &eventTypes, context, &handler)
        guard installed == noErr else { return nil }
        let id = EventHotKeyID(signature: 0x4D494C4B, id: 1)
        let registered = RegisterEventHotKey(UInt32(kVK_Space), modifiers, id, GetApplicationEventTarget(), 0, &hotKey)
        guard registered == noErr else { invalidate(); return nil }
    }

    func invalidate() {
        if let hotKey { UnregisterEventHotKey(hotKey); self.hotKey = nil }
        if let handler { RemoveEventHandler(handler); self.handler = nil }
    }
}
