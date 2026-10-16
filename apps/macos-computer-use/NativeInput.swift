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
        case 2: types = (.otherMouseDown, .otherMouseUp, .center)
        case 3: types = (.rightMouseDown, .rightMouseUp, .right)
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

// The release of the press the helper currently owns, kept on disk so a respawned helper can release what a
// dead generation left down: a synthetic down posted to the session tap survives its poster until logout.
struct OwnedInputJournal {
    let url: URL
    static let standard = OwnedInputJournal(url: FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("Chariox Computer Helper/owned-input.journal"))
    func record(_ release: CGEvent) throws {
        do {
            guard let data = release.data as Data? else { throw Refusal.native }
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: url, options: .atomic)
        } catch { throw tagged(.disabled, "owned-journal-unavailable") }
    }
    func clear() { try? FileManager.default.removeItem(at: url) }
    func pending() -> CGEvent? { (try? Data(contentsOf: url)).flatMap { CGEvent(withDataAllocator: nil, data: $0 as CFData) } }
}

private func ownedControlDown(_ release: CGEvent, _ state: CGEventSourceStateID,
                              _ keyState: (CGEventSourceStateID, CGKeyCode) -> Bool,
                              _ buttonState: (CGEventSourceStateID, CGMouseButton) -> Bool) -> Bool {
    switch release.type {
    case .keyUp: return keyState(state, CGKeyCode(release.getIntegerValueField(.keyboardEventKeycode)))
    case .leftMouseUp: return buttonState(state, .left)
    case .rightMouseUp: return buttonState(state, .right)
    case .otherMouseUp: return buttonState(state, .center)
    default: return false
    }
}

@MainActor final class NativeInputSeat {
    private var ownedRelease: CGEvent?
    private(set) var unresolved = false
    private var running = false
    private let journal: OwnedInputJournal?
    init(journal: OwnedInputJournal? = nil) { self.journal = journal }
    private func released() { ownedRelease = nil; journal?.clear() }
    // Release the control a dead helper generation left down. Only a session-level down with the hardware up
    // is ours to release; a hardware hold belongs to the human and is never touched.
    func recoverStaleRelease(keyState: (CGEventSourceStateID, CGKeyCode) -> Bool = { CGEventSource.keyState($0, key: $1) },
                             buttonState: (CGEventSourceStateID, CGMouseButton) -> Bool = { CGEventSource.buttonState($0, button: $1) },
                             postingAllowed: () -> Bool = { CGPreflightPostEventAccess() && AXIsProcessTrusted() },
                             post: (CGEvent) -> Void) throws {
        guard let release = journal?.pending() else { return }
        if ownedControlDown(release, .combinedSessionState, keyState, buttonState),
           !ownedControlDown(release, .hidSystemState, keyState, buttonState) {
            guard postingAllowed() else { throw tagged(.ownedInput, "owned-release-failed") }
            release.flags = []; post(release)
        }
        journal?.clear()
    }
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
                if [.leftMouseDown, .rightMouseDown, .otherMouseDown, .keyDown].contains(event.type), let release = plan.events.last {
                    try journal?.record(release)
                    ownedRelease = release
                }
                post(event)
                if [.leftMouseUp, .rightMouseUp, .otherMouseUp, .keyUp].contains(event.type) { released() }
                // Let the listen-only tap fence the next event and pending Actions.
                try await Task.sleep(nanoseconds: ownedRelease == nil ? 5_000_000 : plan.dwellNanoseconds)
                try fence()
            }
        } catch {
            if let release = ownedRelease {
                do { try releaseFence() }
                catch { unresolved = true; throw tagged(.ownedInput, "owned-release-failed") }
                release.flags = []; post(release); released()
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
