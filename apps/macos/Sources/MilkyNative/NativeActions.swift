import AppKit

@MainActor
public final class NativeAppOpener: AppOpening {
    public init() {}

    public func open(_ result: AppResult) async throws {
        switch result.action {
        case .openURL(let url):
            guard NSWorkspace.shared.open(url) else { throw OpenError.openFailed }
        case .open(let url):
            guard NSWorkspace.shared.open(url) else { throw OpenError.destinationOpenFailed }
        case .launch(let url):
            if let running = NSWorkspace.shared.runningApplications.first(where: {
                $0.bundleURL?.standardizedFileURL == url.standardizedFileURL
            }) {
                guard running.activate(options: [.activateAllWindows]) else {
                    throw OpenError.activationFailed
                }
                return
            }
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            _ = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
        }
    }

    private enum OpenError: LocalizedError {
        case activationFailed
        case openFailed
        case destinationOpenFailed
        var errorDescription: String? {
            switch self {
            case .activationFailed: "The application could not be activated. Try opening it from Finder."
            case .openFailed: "The destination could not be opened. Try opening it from System Settings."
            case .destinationOpenFailed: "The item could not be opened. Check that it still exists and try again."
            }
        }
    }
}

actor IconCache {
    static let shared = IconCache()
    private var images: [URL: Data] = [:]
    private var settingsBundles: [String: URL]?

    func data(for result: AppResult) async -> Data? {
        let url: URL?
        switch result.action {
        case .launch(let appURL):
            url = appURL
        case .open(let pathURL):
            url = pathURL
        case .openURL:
            url = await settingsBundleURL(for: result.id)
        }
        guard let url else { return nil }
        if let cached = images[url] { return cached }
        let data = await Task.detached(priority: .utility) {
            guard FileManager.default.fileExists(atPath: url.path) else { return Data?.none }
            let source = NSWorkspace.shared.icon(forFile: url.path)
            // Native icons include many large representations. Rasterize only the
            // 32 pt row icon at 2x before transporting or caching it.
            guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 64,
                pixelsHigh: 64, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
                let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = context
            source.draw(in: NSRect(x: 0, y: 0, width: 64, height: 64),
                        from: .zero, operation: .copy, fraction: 1)
            NSGraphicsContext.restoreGraphicsState()
            return bitmap.representation(using: .png, properties: [:])
        }.value
        if let data { images[url] = data }
        return data
    }

    private func settingsBundleURL(for resultID: String) async -> URL? {
        guard resultID.hasPrefix("settings:") else { return nil }
        let paneID = resultID.dropFirst("settings:".count).split(separator: "#", maxSplits: 1).first.map(String.init)
        guard let paneID, !paneID.isEmpty else { return nil }
        if settingsBundles == nil {
            settingsBundles = await Task.detached(priority: .utility) {
                Self.discoverSettingsBundles()
            }.value
        }
        return settingsBundles?[paneID]
    }

    /// Mirrors the two system extension roots used by the Rust settings index.
    /// The lookup happens once, off the UI thread, and is keyed by the pane ID
    /// already present in each result.
    private static func discoverSettingsBundles() -> [String: URL] {
        let roots = [
            URL(fileURLWithPath: "/System/Library/ExtensionKit/Extensions", isDirectory: true),
            URL(fileURLWithPath: "/System/Applications/System Settings.app/Contents/PlugIns", isDirectory: true),
        ]
        var bundles: [String: URL] = [:]
        for root in roots {
            guard let children = try? FileManager.default.contentsOfDirectory(
                at: root, includingPropertiesForKeys: nil, options: [.skipsHiddenFiles]
            ) else { continue }
            for bundleURL in children where bundleURL.pathExtension == "appex" {
                let infoURL = bundleURL.appendingPathComponent("Contents/Info.plist")
                guard let data = try? Data(contentsOf: infoURL),
                      let info = try? PropertyListSerialization.propertyList(from: data, format: nil),
                      let dictionary = info as? [String: Any],
                      let identifier = dictionary["CFBundleIdentifier"] as? String,
                      let extensionAttributes = dictionary["EXAppExtensionAttributes"] as? [String: Any],
                      extensionAttributes["EXExtensionPointIdentifier"] as? String == "com.apple.Settings.extension.ui",
                      let settingsAttributes = extensionAttributes["SettingsExtensionAttributes"] as? [String: Any],
                      settingsAttributes["allowsXAppleSystemPreferencesURLScheme"] as? Bool == true else { continue }
                if bundles[identifier] == nil { bundles[identifier] = bundleURL }
            }
        }
        return bundles
    }
}
