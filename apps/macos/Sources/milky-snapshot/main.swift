import AppKit
import MilkyNative
import SwiftUI

@main
struct MilkySnapshot {
    @MainActor
    static func main() async throws {
        let options = try SnapshotOptions(arguments: Array(CommandLine.arguments.dropFirst()))
        let frontmostBefore = NSWorkspace.shared.frontmostApplication?.processIdentifier
        let app = NSApplication.shared
        if app.activationPolicy() != .prohibited, !app.setActivationPolicy(.prohibited) {
            throw SnapshotError.activationPolicy
        }

        let state = LauncherState(provider: FixtureSearchProvider(), opener: SnapshotOpener())
        if !options.query.isEmpty {
            state.setQuery(options.query)
            if options.captureDuringSearch {
                try await Task.sleep(for: .milliseconds(250))
            } else {
                try await waitForSearch(state)
            }
        }

        let size = NSSize(width: 640, height: 554)
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.backgroundColor = .clear
        if let appearanceName = options.appearanceName {
            window.appearance = NSAppearance(named: appearanceName)
        }
        let hostingView = NSHostingView(rootView: LauncherView(state: state, fixtures: true, dismiss: {}))
        hostingView.frame = NSRect(origin: .zero, size: size)
        window.contentView = hostingView
        window.contentView?.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(250))
        hostingView.layoutSubtreeIfNeeded()

        let pixelsPerPoint = 2
        guard let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: Int(size.width) * pixelsPerPoint,
            pixelsHigh: Int(size.height) * pixelsPerPoint,
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: 0,
            bitsPerPixel: 0
        ) else {
            throw SnapshotError.bitmap
        }
        bitmap.size = size
        hostingView.cacheDisplay(in: hostingView.bounds, to: bitmap)
        guard let png = bitmap.representation(using: .png, properties: [:]) else {
            throw SnapshotError.encoding
        }

        try FileManager.default.createDirectory(
            at: options.outputURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try png.write(to: options.outputURL, options: .atomic)

        let frontmostAfter = NSWorkspace.shared.frontmostApplication?.processIdentifier
        guard frontmostBefore == frontmostAfter else {
            throw SnapshotError.focusChanged
        }
        print("Saved \(options.outputURL.path) without changing the foreground app.")
    }

    @MainActor
    private static func waitForSearch(_ state: LauncherState) async throws {
        for _ in 0..<300 {
            if !state.isSearching { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw SnapshotError.searchTimedOut
    }
}

@MainActor
private struct SnapshotOpener: AppOpening {
    func open(_ result: AppResult) async throws {}
}

private struct SnapshotOptions {
    let query: String
    let outputURL: URL
    let appearanceName: NSAppearance.Name?
    let captureDuringSearch: Bool

    init(arguments: [String]) throws {
        var query = ""
        var outputURL = URL(fileURLWithPath: ".build/snapshots/milky.png", relativeTo: URL(fileURLWithPath: FileManager.default.currentDirectoryPath))
        var appearanceName: NSAppearance.Name?
        var captureDuringSearch = false
        var index = 0

        while index < arguments.count {
            let argument = arguments[index]
            if argument == "--during-search" {
                captureDuringSearch = true
                index += 1
                continue
            }
            guard index + 1 < arguments.count else { throw SnapshotError.usage }
            let value = arguments[index + 1]
            switch argument {
            case "--query": query = value
            case "--output": outputURL = URL(fileURLWithPath: value)
            case "--appearance":
                switch value {
                case "light": appearanceName = .aqua
                case "dark": appearanceName = .darkAqua
                default: throw SnapshotError.usage
                }
            default: throw SnapshotError.usage
            }
            index += 2
        }

        self.query = query
        self.outputURL = outputURL.standardizedFileURL
        self.appearanceName = appearanceName
        self.captureDuringSearch = captureDuringSearch
    }
}

private enum SnapshotError: LocalizedError {
    case usage
    case activationPolicy
    case bitmap
    case encoding
    case focusChanged
    case searchTimedOut

    var errorDescription: String? {
        switch self {
        case .usage:
            return "Usage: milky-snapshot [--query TEXT] [--during-search] [--appearance light|dark] [--output PATH]"
        case .activationPolicy:
            return "Could not set the snapshot process to a non-activating policy."
        case .bitmap:
            return "Could not create the snapshot bitmap."
        case .encoding:
            return "Could not encode the snapshot as PNG."
        case .focusChanged:
            return "The foreground application changed while rendering; snapshot discarded."
        case .searchTimedOut:
            return "Fixture search did not finish before the snapshot timeout."
        }
    }
}
