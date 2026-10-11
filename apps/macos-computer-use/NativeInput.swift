import AppKit

struct NativeInputPlan {
    let point: CGPoint?
    let events: [CGEvent]
    let typing: Bool
    let dwellNanoseconds: UInt64
    init(point: CGPoint?, events: [CGEvent], typing: Bool, dwellNanoseconds: UInt64 = 5_000_000) {
        self.point = point; self.events = events; self.typing = typing; self.dwellNanoseconds = dwellNanoseconds
    }
}

// Existing shared Computer input kinds. Long holds, raw keycodes and shortcuts stay
// refused until granted fatal-death and inert-key acceptance has passed.
func nativeInputPlan(_ input: [String: Any], surface: NativeSurface) throws -> NativeInputPlan {
    let source = CGEventSource(stateID: .privateState)
    func point() throws -> CGPoint {
        guard let x = input["x"] as? UInt32, let y = input["y"] as? UInt32 else { throw tagged(.arguments, "invalid-native-point") }
        return try surface.point(x: x, y: y)
    }
    func mouse(_ type: CGEventType, at point: CGPoint, button: CGMouseButton) throws -> CGEvent {
        guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: button) else {
            throw tagged(.native, "mouse-event-create")
        }
        return event
    }
    switch input["kind"] as? String {
    case "move":
        let location = try point()
        return NativeInputPlan(point: location, events: [try mouse(.mouseMoved, at: location, button: .left)], typing: false)
    case "click":
        let location = try point()
        let button = input["button"] as? Int ?? 1
        let types: (CGEventType, CGEventType, CGMouseButton)
        switch button {
        case 1: types = (.leftMouseDown, .leftMouseUp, .left)
        case 2: types = (.rightMouseDown, .rightMouseUp, .right)
        case 3: types = (.otherMouseDown, .otherMouseUp, .center)
        default: throw tagged(.arguments, "invalid-native-button")
        }
        return NativeInputPlan(point: location, events: try [types.0, types.1].map { try mouse($0, at: location, button: types.2) }, typing: false)
    case "scroll":
        let location = try point()
        guard let steps = input["steps"] as? Int, steps != 0, (-100...100).contains(steps),
              let event = CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 1,
                                  wheel1: Int32(-steps * 40), wheel2: 0, wheel3: 0) else { throw tagged(.arguments, "invalid-native-scroll") }
        event.location = location
        return NativeInputPlan(point: location, events: [event], typing: false)
    case "key", "hold":
        let keys: [String: CGKeyCode] = ["left": 123, "right": 124, "down": 125, "up": 126,
            "home": 115, "end": 119, "escape": 53, "backspace": 51, "delete": 117]
        guard let key = input["key"] as? String, let code = keys[key.lowercased()] else { throw tagged(.arguments, "unsupported-native-key") }
        var dwell: UInt64 = 5_000_000
        if input["kind"] as? String == "hold" {
            guard let milliseconds = input["duration_ms"] as? Int, (1...100).contains(milliseconds) else {
                throw tagged(.arguments, "native-hold-budget")
            }
            dwell = UInt64(milliseconds) * 1_000_000
        }
        let events = try [true, false].map { down -> CGEvent in
            guard let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down) else { throw Refusal.native }
            return event
        }
        return NativeInputPlan(point: nil, events: events, typing: true, dwellNanoseconds: dwell)
    default: throw tagged(.arguments, "unsupported-native-input")
    }
}

@MainActor final class NativeInputSeat {
    private var ownedRelease: CGEvent?
    private(set) var unresolved = false
    private var running = false
    func dispatch(_ plan: NativeInputPlan, fence: () throws -> Void,
                  releaseFence: () throws -> Void, post: (CGEvent) -> Void) async throws {
        guard !unresolved, ownedRelease == nil else { throw tagged(.ownedInput, "owned-release-failed") }
        guard !running else { throw tagged(.disabled, "native-seat-busy") }
        running = true
        defer { running = false }
        do {
            for event in plan.events {
                try fence()
                event.flags = []
                // Record before posting so orderly cancellation cannot miss a down.
                if [.leftMouseDown, .rightMouseDown, .otherMouseDown, .keyDown].contains(event.type) {
                    ownedRelease = plan.events.last
                }
                post(event)
                if [.leftMouseUp, .rightMouseUp, .otherMouseUp, .keyUp].contains(event.type) { ownedRelease = nil }
                // Let the listen-only tap fence the next event and pending Actions.
                try await Task.sleep(nanoseconds: ownedRelease == nil ? 5_000_000 : plan.dwellNanoseconds)
                try fence()
            }
        } catch {
            if let release = ownedRelease {
                do { try releaseFence() }
                catch { unresolved = true; throw tagged(.ownedInput, "owned-release-failed") }
                release.flags = []; post(release); ownedRelease = nil
            }
            throw error
        }
    }
}

@MainActor struct NativeReleaseFence<Element> {
    let destination: NativeDestinationFence<Element>
    let request: Request
    let element: Element
    let plan: NativeInputPlan
    let geometry: ClickGeometry
    let observer: any PhysicalInputObserver
    let foreignHeld: () -> Bool
    let postingAllowed: () -> Bool
    func check() throws {
        try observer.checkHealth()
        // A cancelled Action can release only to its still-admitted destination.
        // Observer health, target focus/visibility/geometry and posting permission
        // stay mandatory even though the cancelled Action's authority is retired.
        try destination.check(request, element: element, typing: plan.typing,
            clickGeometry: geometry, eventLocation: plan.point ?? geometry.location,
            physicalKey: plan.typing, checkAuthority: false)
        try observer.checkHealth()
        guard !observer.fence.foreignPressObserved, !foreignHeld() else { throw tagged(.disabled, "foreign-input-held") }
        guard postingAllowed() else { throw Refusal.permission }
    }
}

extension MacSource {
    @MainActor func performPhysical(_ plan: NativeInputPlan, request: Request,
                                   observer: any PhysicalInputObserver, seat: NativeInputSeat) async throws -> [String] {
        // M4 protection is not proven for real apps. Only the exact signed fixture
        // can enter this experimental physical path.
        guard !request.ownerWindow else { throw tagged(.disabled, "physical-owner-window-unavailable") }
        try observer.check()
        guard !foreignInputHeld() else { throw tagged(.disabled, "foreign-input-held") }
        var request = request
        request.clickPoint = plan.point
        let element = try target(request)
        let geometry = try self.geometry(request, element: element)
        let release = NativeReleaseFence(destination: destinationFence(for: request), request: request,
            element: element, plan: plan, geometry: geometry, observer: observer,
            foreignHeld: { foreignInputHeld(excludingOwned: plan.events.last) },
            postingAllowed: { CGPreflightPostEventAccess() && AXIsProcessTrusted() })
        try await seat.dispatch(plan, fence: {
            try observer.check()
            try checkPermission(.click)
            try self.fence(request, element: element, typing: plan.typing,
                clickGeometry: plan.point == nil ? nil : geometry, eventLocation: plan.point, physicalKey: plan.typing)
            guard CGPreflightPostEventAccess() else { throw tagged(.permission, "post-permission-denied") }
        }, releaseFence: { try release.check()
        }, post: { event in
            observer.fence.tag(event)
            event.post(tap: .cgSessionEventTap)
        })
        return [inputReceipt(path: "CGEventSession", observed: "application completion unproven")]
    }
}
