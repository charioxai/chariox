import AppKit

// AppKit has a bottom-left screen origin; AX and CG have a top-left origin.
// Both use points. Backing scale belongs to bitmap pixels, never this mapping.
func quartzPoint(fromCocoa point: CGPoint, primaryTop: CGFloat) -> CGPoint {
    CGPoint(x: point.x, y: primaryTop - point.y)
}

func currentPointerPoint() throws -> CGPoint {
    let cocoa = NSEvent.mouseLocation
    guard let primary = NSScreen.screens.first else { throw tagged(.native, "pointer-primary-screen") }
    let point = quartzPoint(fromCocoa: cocoa, primaryTop: primary.frame.maxY)
    guard point.x.isFinite, point.y.isFinite else { throw tagged(.native, "nonfinite-pointer") }
    return point
}
