import AppKit
import QuickLookUI
import SwiftUI

struct FileInspectionView: View {
    let result: AppResult
    let close: () -> Void

    private var fileURL: URL? {
        guard case .open(let url) = result.action, result.kind == .file else { return nil }
        return url
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "doc.text.magnifyingglass")
                    .font(.system(size: 22))
                    .foregroundStyle(LauncherTheme.secondary)
                VStack(alignment: .leading, spacing: 3) {
                    Text(result.title)
                        .font(.system(size: 16, weight: .semibold))
                        .lineLimit(1)
                    Text(fileURL?.path ?? result.subtitle)
                        .font(LauncherTheme.subtitle)
                        .foregroundStyle(LauncherTheme.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                }
                Spacer(minLength: 8)
                Button("Back") { close() }.buttonStyle(.plain)
            }
            .padding(LauncherTheme.inset)
            Divider()
            if let fileURL, FileManager.default.fileExists(atPath: fileURL.path) {
                QuickLookFilePreview(url: fileURL)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityLabel("Preview of \(result.title)")
            } else {
                VStack(spacing: 8) {
                    Image(systemName: "doc.questionmark").font(.system(size: 30))
                    Text("File unavailable").font(.system(size: 16, weight: .medium))
                    Text("The file may have moved or been deleted.")
                        .font(LauncherTheme.subtitle)
                }
                .foregroundStyle(LauncherTheme.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
}

private struct QuickLookFilePreview: NSViewRepresentable {
    let url: URL

    func makeNSView(context: Context) -> QLPreviewView {
        let view = QLPreviewView(frame: .zero)
        view?.autostarts = true
        view?.previewItem = url as NSURL
        return view!
    }

    func updateNSView(_ view: QLPreviewView, context: Context) {
        view.previewItem = url as NSURL
    }

    static func dismantleNSView(_ view: QLPreviewView, coordinator: ()) {
        view.close()
    }
}
