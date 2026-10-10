import AppKit

enum InputPath: String { case axPress, axScrollValue, axTextClick, pidText }

enum TextClickResolution: Equatable {
    case selection(Int)
}

func visibleWindowFrame(window: CGRect, element: CGRect, displays: [CGRect], point: CGPoint?) throws -> CGRect {
    let visible = displays.map { window.intersection($0) }
        .filter { !$0.intersection(element).isNull && !$0.intersection(element).isEmpty }
    if let point {
        guard point.x.isFinite, point.y.isFinite,
              let frame = visible.first(where: { $0.intersection(element).contains(point) }) else { throw tagged(.target, "point-outside-visible-target") }
        return frame
    }
    guard let frame = visible.max(by: {
        let a = $0.intersection(element), b = $1.intersection(element)
        return a.width * a.height < b.width * b.height
    }) else { throw tagged(.target, "no-visible-target") }
    return frame
}

func resolveTextClick(at point: CGPoint, rangeAtPoint: (CGPoint) throws -> CFRange?) throws -> TextClickResolution {
    guard point.x.isFinite, point.y.isFinite else { throw tagged(.target, "nonfinite-point") }
    // App-scoped AX hit-testing cannot detect a foreign covering window.
    // Refuse unavailable range metadata; no global mouse fallback is admitted.
    guard let range = try rangeAtPoint(point) else { throw tagged(.target, "no-range-at-point") }
    // AX returns the composed-character range. Collapse at its start, never
    // split a surrogate pair or select the character under the pointer.
    guard range.location >= 0, range.length >= 0,
          range.location <= Int.max - range.length else { throw tagged(.target, "malformed-position-range") }
    return .selection(range.location)
}

struct ClickGeometry: Equatable {
    let windowBounds: CGRect
    let elementBounds: CGRect
    let visibleWindowBounds: CGRect
    let requestedPoint: CGPoint?
    init(windowBounds: CGRect, elementBounds: CGRect, visibleWindowBounds: CGRect? = nil,
         requestedPoint: CGPoint? = nil) {
        self.windowBounds = windowBounds; self.elementBounds = elementBounds
        self.visibleWindowBounds = visibleWindowBounds ?? windowBounds
        self.requestedPoint = requestedPoint
    }
    var visibleBounds: CGRect { elementBounds.intersection(windowBounds).intersection(visibleWindowBounds) }
    var location: CGPoint { requestedPoint ?? CGPoint(x: visibleBounds.midX, y: visibleBounds.midY) }

    func checkedLocation(_ eventLocation: CGPoint, current: ClickGeometry) throws -> CGPoint {
        guard current == self, eventLocation == location,
              eventLocation.x.isFinite, eventLocation.y.isFinite,
              !visibleBounds.isNull, !visibleBounds.isEmpty,
              visibleBounds.contains(eventLocation) else { throw tagged(.target, "stale-point") }
        return eventLocation
    }
}

func checkPointerSnapshot(_ request: Request, currentPoint: CGPoint) throws {
    guard request.clickAtPointer else { return }
    guard let saved = request.clickPoint else { throw tagged(.target, "pointer-not-snapshotted") }
    guard saved == currentPoint else { throw tagged(.target, "stale-pointer") }
}

func dispatchInputEvents(_ events: [CGEvent], fence: (CGEvent) throws -> Void,
                         releaseFence: (CGEvent) throws -> Void,
                         releaseAllowed: () -> Bool, post: (CGEvent) -> Void) throws {
    var release: CGEvent?
    do {
        for event in events {
            try fence(event)
            guard releaseAllowed() else { throw tagged(.permission, "post-event-permission") }
            event.flags = []; post(event)
            if [.leftMouseDown, .rightMouseDown, .otherMouseDown, .keyDown].contains(event.type) { release = events.last }
            if [.leftMouseUp, .rightMouseUp, .otherMouseUp, .keyUp].contains(event.type) { release = nil }
        }
    } catch {
        guard let release else { throw error }
        // Cleanup of an owned pair has its own fence. Fatal death remains unproven.
        do {
            try releaseFence(release)
            guard releaseAllowed() else { throw tagged(.permission, "release-event-permission") }
        } catch { throw tagged(.ownedInput, "owned-release-failed") }
        release.flags = []; post(release)
        throw error
    }
}

func inputPath(_ operation: Operation, role: String) throws -> InputPath {
    switch operation {
    case .click where role == kAXButtonRole: return .axPress
    case .scroll where role == kAXScrollAreaRole: return .axScrollValue
    case .click where [kAXTextFieldRole, kAXTextAreaRole].contains(role): return .axTextClick
    case .text where [kAXTextFieldRole, kAXTextAreaRole].contains(role): return .pidText
    case .click, .scroll, .text: throw tagged(.target, "unsupported-input-role")
    default: throw tagged(.arguments, "unsupported-input-operation")
    }
}

func nextScrollValue(_ value: Double) throws -> Double {
    guard value.isFinite, (0...1).contains(value), value < 1 else { throw tagged(.target, "scroll-at-limit") }
    // A single bounded step toward the bottom, independent of natural scrolling.
    return min(1, value + 0.05)
}

func inputReceipt(path: String, observed: String? = nil) -> String {
    "dispatched; path=\(path); " + (observed ?? "application completion unproven")
}
