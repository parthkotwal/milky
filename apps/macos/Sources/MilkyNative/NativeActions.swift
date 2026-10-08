import AppKit

@MainActor
public final class NativeAppOpener: AppOpening {
    public init() {}

    public func open(_ result: AppResult) async throws {
        switch result.action {
        case .openURL(let url):
            guard NSWorkspace.shared.open(url) else { throw OpenError.openFailed }
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
        var errorDescription: String? {
            switch self {
            case .activationFailed: "The application could not be activated. Try opening it from Finder."
            case .openFailed: "The destination could not be opened. Try opening it from System Settings."
            }
        }
    }
}

actor IconCache {
    static let shared = IconCache()
    private var images: [URL: Data] = [:]

    func data(for url: URL) async -> Data? {
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
}
