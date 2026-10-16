import SwiftUI

/// Fixture-only studies of a small drawn mark and selected-row treatment.
/// The live launcher uses LauncherView without an identity value.
public enum CowIdentity: String, CaseIterable, Sendable {
    case spot
    case ear
    case muzzle

    public var title: String {
        switch self {
        case .spot: "Spot + wash"
        case .ear: "Ear + outline"
        case .muzzle: "Muzzle + inverse"
        }
    }
}

struct CowPalette {
    let surface: Color
    let ink: Color
    let secondary: Color
    let pink: Color

    init(dark: Bool) {
        surface = dark ? Color(red: 0.055, green: 0.055, blue: 0.055) : .white
        ink = dark ? .white : .black
        secondary = dark ? Color(red: 0.76, green: 0.76, blue: 0.76)
                         : Color(red: 0.31, green: 0.31, blue: 0.31)
        pink = dark ? Color(red: 0.97, green: 0.65, blue: 0.76)
                    : Color(red: 0.78, green: 0.33, blue: 0.49)
    }
}

/// Drawn directly in the empty 15 pt strip at the panel's upper edge.
/// The mark has no hit target or spoken content.
struct CowEdgeMotif: View {
    let identity: CowIdentity
    let palette: CowPalette

    var body: some View {
        GeometryReader { geometry in
            let width = geometry.size.width
            let height = geometry.size.height
            switch identity {
            case .spot:
                Path { path in
                    path.move(to: CGPoint(x: 2, y: 0))
                    path.addCurve(to: CGPoint(x: width * 0.44, y: height * 0.88),
                                  control1: CGPoint(x: width * 0.03, y: height * 0.80),
                                  control2: CGPoint(x: width * 0.20, y: height * 1.12))
                    path.addCurve(to: CGPoint(x: width * 0.75, y: height * 0.45),
                                  control1: CGPoint(x: width * 0.60, y: height * 0.65),
                                  control2: CGPoint(x: width * 0.61, y: height * 0.11))
                    path.addCurve(to: CGPoint(x: width - 2, y: 0),
                                  control1: CGPoint(x: width * 0.90, y: height * 0.75),
                                  control2: CGPoint(x: width * 0.97, y: height * 0.28))
                    path.closeSubpath()
                }
                .fill(palette.ink)
                Circle().fill(palette.pink)
                    .frame(width: 4, height: 4)
                    .position(x: width * 0.54, y: height * 0.38)
            case .ear:
                Path { path in
                    path.move(to: CGPoint(x: 0, y: 1))
                    path.addQuadCurve(to: CGPoint(x: width * 0.43, y: height * 0.81),
                                      control: CGPoint(x: width * 0.08, y: height * 1.25))
                    path.addQuadCurve(to: CGPoint(x: width * 0.50, y: 1),
                                      control: CGPoint(x: width * 0.45, y: height * 0.15))
                    path.move(to: CGPoint(x: width, y: 1))
                    path.addQuadCurve(to: CGPoint(x: width * 0.57, y: height * 0.81),
                                      control: CGPoint(x: width * 0.92, y: height * 1.25))
                    path.addQuadCurve(to: CGPoint(x: width * 0.50, y: 1),
                                      control: CGPoint(x: width * 0.55, y: height * 0.15))
                }
                .stroke(palette.ink, style: StrokeStyle(lineWidth: 2, lineCap: .round,
                                                        lineJoin: .round))
                Path { path in
                    path.move(to: CGPoint(x: width * 0.11, y: height * 0.32))
                    path.addQuadCurve(to: CGPoint(x: width * 0.35, y: height * 0.66),
                                      control: CGPoint(x: width * 0.18, y: height * 0.76))
                }
                .stroke(palette.pink, style: StrokeStyle(lineWidth: 1.5, lineCap: .round))
            case .muzzle:
                Path { path in
                    path.move(to: CGPoint(x: 2, y: 0))
                    path.addCurve(to: CGPoint(x: width - 2, y: 0),
                                  control1: CGPoint(x: 1, y: height * 1.25),
                                  control2: CGPoint(x: width - 1, y: height * 1.25))
                }
                .stroke(palette.ink, style: StrokeStyle(lineWidth: 2, lineCap: .round))
                HStack(spacing: 16) {
                    Ellipse().fill(palette.pink).frame(width: 4, height: 3)
                    Ellipse().fill(palette.pink).frame(width: 4, height: 3)
                }
                .position(x: width / 2, y: height * 0.48)
            }
        }
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }
}
