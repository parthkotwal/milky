import AppKit
import XCTest
@testable import MilkyNative

final class InvocationTests: XCTestCase {
    func testDismissConsumesFocusTargetBeforeReentrantNotification() {
        var session = InvocationSession()
        session.begin(previousPID: 123)
        let generation = session.generation
        let returnTarget = session.dismiss(restoreFocus: true, ownsFocus: true)
        XCTAssertEqual(returnTarget, 123)
        XCTAssertFalse(session.isPresented)
        XCTAssertGreaterThan(session.generation, generation)
        XCTAssertNil(session.dismiss(restoreFocus: true, ownsFocus: true))
    }

    func testAppSwitchAndActionNeverStealFocusBack() {
        var session = InvocationSession()
        session.begin(previousPID: 123)
        XCTAssertNil(session.dismiss(restoreFocus: true, ownsFocus: false))
        session.begin(previousPID: 456)
        XCTAssertNil(session.dismiss(restoreFocus: false, ownsFocus: true))
        session.begin(previousPID: nil)
        XCTAssertNil(session.dismiss(restoreFocus: true, ownsFocus: true))
    }

    func testRepeatedShowRetainsOriginalReturnTargetAndNewSessionReplacesIt() {
        var session = InvocationSession()
        session.begin(previousPID: 123)
        let token = session.generation
        session.begin(previousPID: 456)
        XCTAssertEqual(session.generation, token)
        XCTAssertEqual(session.dismiss(restoreFocus: true, ownsFocus: true), 123)
        session.begin(previousPID: 789)
        XCTAssertEqual(session.dismiss(restoreFocus: true, ownsFocus: true), 789)
    }

    func testHeldShortcutOnlyInvokesOnceUntilRelease() {
        var gate = ShortcutPressGate()
        XCTAssertTrue(gate.keyDown())
        for _ in 0..<20 { XCTAssertFalse(gate.keyDown()) }
        gate.keyUp()
        XCTAssertTrue(gate.keyDown())
        gate.keyUp()
        gate.keyUp()
        XCTAssertTrue(gate.keyDown())
    }

    func testPlacementFitsSmallNegativeOriginAndPortraitDisplays() {
        for visible in [
            CGRect(x: 0, y: 0, width: 1440, height: 875),
            CGRect(x: -1920, y: -200, width: 1920, height: 1080),
            CGRect(x: 1440, y: 0, width: 800, height: 1280),
            CGRect(x: 0, y: 0, width: 280, height: 220),
        ] {
            let frame = PanelGeometry.frame(in: visible)
            XCTAssertTrue(visible.contains(frame), "Panel escaped available frame: \(visible)")
            XCTAssertLessThanOrEqual(frame.width, 640)
            XCTAssertLessThanOrEqual(frame.height, 554)
            XCTAssertEqual(frame.midX, visible.midX, accuracy: 0.001)
        }
    }
}

@MainActor final class LauncherContrastTests: XCTestCase {
    func testSecondaryTextAndSelectionStripeMeetContrastInBothAppearances() throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let appearance = try XCTUnwrap(NSAppearance(named: name))
            var ratios: [Double] = []
            appearance.performAsCurrentDrawingAppearance {
                let surface = NSColor.windowBackgroundColor.usingColorSpace(.sRGB)!
                let text = LauncherTheme.secondaryTextColor.usingColorSpace(.sRGB)!
                let accent = NSColor.controlAccentColor.usingColorSpace(.sRGB)!
                let base = [surface.redComponent, surface.greenComponent, surface.blueComponent]
                let selected = zip(base, [accent.redComponent, accent.greenComponent, accent.blueComponent]).map { $0 * 0.87 + $1 * 0.13 }
                for background in [base, selected] {
                    let foreground = zip(background, [text.redComponent, text.greenComponent, text.blueComponent]).map {
                        $0 * (1 - text.alphaComponent) + $1 * text.alphaComponent
                    }
                    func luminance(_ rgb: [CGFloat]) -> Double {
                        let linear = rgb.map { value -> Double in
                            let c = Double(value)
                            return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
                        }
                        return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
                    }
                    let a = luminance(foreground), b = luminance(background)
                    ratios.append((max(a, b) + 0.05) / (min(a, b) + 0.05))
                }
            }
            for ratio in ratios { XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(name): \(ratio)") }
        }
    }
}
