import AppKit

struct NativeSurface: Equatable {
    let bounds: CGRect
    let scale: CGFloat
    var width: Int { Int(bounds.width * scale) }
    var height: Int { Int(bounds.height * scale) }
    func point(x: UInt32, y: UInt32) throws -> CGPoint {
        guard scale.isFinite, scale > 0, width > 0, height > 0,
              Int(x) < width, Int(y) < height else { throw tagged(.target, "point-outside-surface") }
        return CGPoint(x: bounds.minX + CGFloat(x) / scale, y: bounds.minY + CGFloat(y) / scale)
    }
}

func nativeSurface(_ scope: Request) throws -> NativeSurface {
    let bounds = try MacSource().windowBounds(scope)
    let primaryTop = NSScreen.screens.first?.frame.maxY ?? 0
    let matches = NSScreen.screens.filter { screen in
        let quartz = CGRect(x: screen.frame.minX, y: primaryTop - screen.frame.maxY,
                            width: screen.frame.width, height: screen.frame.height)
        return quartz.contains(bounds)
    }
    // A window spanning displays of different scales needs M2 transform revision.
    guard matches.count == 1, let screen = matches.first, [CGFloat(1), CGFloat(2)].contains(screen.backingScaleFactor),
          bounds.width > 0, bounds.height > 0, bounds.width * screen.backingScaleFactor <= 4096,
          bounds.height * screen.backingScaleFactor <= 4096 else { throw tagged(.target, "surface-geometry-unproven") }
    return NativeSurface(bounds: bounds, scale: screen.backingScaleFactor)
}
