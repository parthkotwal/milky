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
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

@MainActor
private final class LauncherDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private let fixtures = CommandLine.arguments.contains("--fixtures")
    private let rust = RustSearchProvider()
    private lazy var state = LauncherState(provider: fixtures ? FixtureSearchProvider() : rust, opener: NativeAppOpener())
    private var preparation: Task<Void, Never>?
    private var terminating = false
    private var displayName: String { fixtures ? "Milky — Fixtures" : "Milky" }
    private var panel: LauncherPanel!
    private var statusItem: NSStatusItem!
    private var shortcut: GlobalShortcut?
    private var previousApp: NSRunningApplication?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)
        installMainMenu()
        panel = LauncherPanel(contentRect: NSRect(x: 0, y: 0, width: 640, height: 554),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        panel.title = displayName
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true
        panel.isMovableByWindowBackground = true
        panel.isReleasedWhenClosed = false
        panel.delegate = self
        panel.contentView = NSHostingView(rootView: LauncherView(state: state, fixtures: fixtures) { [weak self] in
            self?.dismiss(restoreFocus: true)
        })
        if CommandLine.arguments.contains("--appearance=light") { panel.appearance = NSAppearance(named: .aqua) }
        if CommandLine.arguments.contains("--appearance=dark") { panel.appearance = NSAppearance(named: .darkAqua) }
        state.onOpened = { [weak self] in self?.dismiss(restoreFocus: false) }

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem.button?.image = NSImage(systemSymbolName: "magnifyingglass.circle", accessibilityDescription: displayName)
        let menu = NSMenu()
        let showItem = menu.addItem(withTitle: "Show \(displayName)", action: #selector(showFromMenu), keyEquivalent: "")
        showItem.target = self
        menu.addItem(.separator())
        let quit = menu.addItem(withTitle: "Quit Milky", action: #selector(quitApp), keyEquivalent: "q")
        quit.target = self
        statusItem.menu = menu

        let alternate = CommandLine.arguments.contains("--shortcut=command-shift-space")
        shortcut = GlobalShortcut(modifiers: UInt32(alternate ? cmdKey | shiftKey : controlKey | optionKey)) { [weak self] in
            guard let self else { return }
            if self.panel.isVisible { self.dismiss(restoreFocus: true) } else { self.show() }
        }
        let label = alternate ? "⇧⌘Space" : "⌃⌥Space"
        if shortcut == nil {
            let item = NSMenuItem(title: "Shortcut unavailable — use Show Milky", action: nil, keyEquivalent: "")
            menu.insertItem(item, at: 1)
            fputs("Milky: global shortcut registration failed; use the menu bar item.\n", stderr)
        } else {
            showItem.title = "Show \(displayName)  \(label)"
        }
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
        NSApp.mainMenu = main
    }

    @objc private func showFromMenu() { show() }
    @objc private func quitApp() { NSApp.terminate(nil) }

    private func show() {
        guard !terminating else { return }
        if !panel.isVisible {
            let front = NSWorkspace.shared.frontmostApplication
            if front?.processIdentifier != ProcessInfo.processInfo.processIdentifier { previousApp = front }
            state.beginSession()
        }
        let mouse = NSEvent.mouseLocation
        let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) } ?? NSScreen.main
        if let visible = screen?.visibleFrame {
            let width = min(640, max(300, visible.width - 32))
            let height = min(554, max(250, visible.height - 32))
            panel.setFrame(NSRect(x: visible.midX - width / 2,
                                  y: visible.minY + (visible.height - height) * 0.64,
                                  width: width, height: height), display: true)
        }
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
        panel.contentView?.layoutSubtreeIfNeeded()
        focusSearch()
        // SwiftUI may attach its NSTextField after the initial hosting layout.
        DispatchQueue.main.async { [weak self] in self?.focusSearch() }
    }

    private func focusSearch() {
        guard panel.isVisible, panel.isKeyWindow else { return }
        if let field = findField(panel.contentView) { panel.makeFirstResponder(field) }
    }

    private func findField(_ view: NSView?) -> NSTextField? {
        if let field = view as? NSTextField { return field }
        for child in view?.subviews ?? [] {
            if let field = findField(child) { return field }
        }
        return nil
    }

    private func dismiss(restoreFocus: Bool) {
        let ownedFocus = NSWorkspace.shared.frontmostApplication?.processIdentifier == ProcessInfo.processInfo.processIdentifier
        state.dismiss()
        panel.orderOut(nil)
        if restoreFocus, ownedFocus { previousApp?.activate(options: []) }
        previousApp = nil
    }

    func windowDidBecomeKey(_ notification: Notification) {
        DispatchQueue.main.async { [weak self] in self?.focusSearch() }
    }

    func windowDidResignKey(_ notification: Notification) {
        // Opening may transfer focus before NSWorkspace's completion arrives.
        if panel.isVisible, !state.isOpening { dismiss(restoreFocus: false) }
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
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
    }
}

@MainActor
private final class GlobalShortcut {
    private var hotKey: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private let action: () -> Void

    init?(modifiers: UInt32, action: @escaping () -> Void) {
        self.action = action
        var eventType = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let context = Unmanaged.passUnretained(self).toOpaque()
        let installed = InstallEventHandler(GetApplicationEventTarget(), { _, _, context in
            guard let context else { return OSStatus(eventNotHandledErr) }
            MainActor.assumeIsolated {
                Unmanaged<GlobalShortcut>.fromOpaque(context).takeUnretainedValue().action()
            }
            return noErr
        }, 1, &eventType, context, &handler)
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
