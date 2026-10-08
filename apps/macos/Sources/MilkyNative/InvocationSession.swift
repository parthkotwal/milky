import Foundation

/// Separate session ownership from NSPanel visibility: AppKit can change
/// visibility and send resign-key callbacks while dismissal is still executing.
public struct InvocationSession {
    public private(set) var isPresented = false
    public private(set) var generation = 0
    private var returnPID: Int32?

    public init() {}

    public mutating func begin(previousPID: Int32?) {
        guard !isPresented else { return }
        generation += 1
        isPresented = true
        returnPID = previousPID
    }

    /// Consume the return target before calling AppKit. Reentrant dismissal is inert.
    public mutating func dismiss(restoreFocus: Bool, ownsFocus: Bool) -> Int32? {
        guard isPresented else { return nil }
        let target = restoreFocus && ownsFocus ? returnPID : nil
        isPresented = false
        returnPID = nil
        generation += 1
        return target
    }
}

public enum PanelGeometry {
    public static func frame(in visible: CGRect) -> CGRect {
        let inset = min(16, max(0, min(visible.width, visible.height) / 4))
        let width = min(640, max(1, visible.width - 2 * inset))
        let height = min(554, max(1, visible.height - 2 * inset))
        return CGRect(x: visible.midX - width / 2,
                      y: visible.minY + (visible.height - height) * 0.64,
                      width: width, height: height)
    }
}

public struct ShortcutPressGate {
    private var pressed = false
    public init() {}
    public mutating func keyDown() -> Bool {
        guard !pressed else { return false }
        pressed = true
        return true
    }
    public mutating func keyUp() { pressed = false }
}
