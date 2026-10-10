import AppKit
import Carbon

// Value-free local fence. Keycodes and typed human text never enter this state.
@MainActor final class PhysicalHumanFence {
    private let marker: Int64
    private let pid: Int64
    var observerReady = false
    private(set) var paused = false
    private var expected: [(CGEventType, CGPoint, TimeInterval)] = []
    init(marker: Int64 = Int64.random(in: 1...Int64.max), pid: Int64 = Int64(getpid())) {
        self.marker = marker; self.pid = pid
    }
    func tag(_ event: CGEvent) {
        event.setIntegerValueField(.eventSourceUserData, value: marker)
        event.setIntegerValueField(.eventSourceUnixProcessID, value: pid)
        let now = ProcessInfo.processInfo.systemUptime
        expected.removeAll { $0.2 < now }
        guard expected.count < 64 else { paused = true; return }
        expected.append((event.type, event.location, now + 0.5))
    }
    func observe(_ type: CGEventType, event: CGEvent) {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            observerReady = false; paused = true; return
        }
        let now = ProcessInfo.processInfo.systemUptime
        expected.removeAll { $0.2 < now }
        if event.getIntegerValueField(.eventSourceUserData) == marker,
           event.getIntegerValueField(.eventSourceUnixProcessID) == pid,
           let index = expected.firstIndex(where: { $0.0 == type && $0.1 == event.location }) {
            expected.remove(at: index); return
        }
        paused = true
    }
    func check() throws {
        guard observerReady, !paused else { throw tagged(.disabled, "physical-human-paused") }
    }
    func release(foreignHeld: Bool) throws {
        guard observerReady, !foreignHeld else { throw tagged(.disabled, "physical-human-release-refused") }
        expected.removeAll(); paused = false
    }
}

@MainActor protocol PhysicalInputObserver: AnyObject {
    var fence: PhysicalHumanFence { get }
    func start() throws
    func check() throws
    func stop()
}

@MainActor final class PhysicalHumanObserver: PhysicalInputObserver {
    let fence: PhysicalHumanFence
    private var tap: CFMachPort?
    private var source: CFRunLoopSource?
    init(fence: PhysicalHumanFence) { self.fence = fence }
    func start() throws {
        guard CGPreflightListenEventAccess() else { throw tagged(.permission, "listen-event-access-denied") }
        let types: [CGEventType] = [.mouseMoved, .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp,
            .otherMouseDown, .otherMouseUp, .leftMouseDragged, .rightMouseDragged, .otherMouseDragged,
            .scrollWheel, .keyDown, .keyUp, .flagsChanged]
        let mask = types.reduce(CGEventMask(0)) { $0 | (CGEventMask(1) << $1.rawValue) }
        guard let tap = CGEvent.tapCreate(tap: .cgSessionEventTap, place: .tailAppendEventTap,
            options: .listenOnly, eventsOfInterest: mask, callback: { _, type, event, context in
                guard let context else { return Unmanaged.passUnretained(event) }
                MainActor.assumeIsolated {
                    Unmanaged<PhysicalHumanObserver>.fromOpaque(context).takeUnretainedValue().fence.observe(type, event: event)
                }
                return Unmanaged.passUnretained(event)
            }, userInfo: Unmanaged.passUnretained(self).toOpaque()),
            let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0) else {
            throw tagged(.permission, "listen-event-tap-unavailable")
        }
        self.tap = tap; self.source = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        fence.observerReady = CGEvent.tapIsEnabled(tap: tap)
        try fence.check()
    }
    func check() throws {
        guard let tap, CGEvent.tapIsEnabled(tap: tap), !IsSecureEventInputEnabled() else {
            fence.observerReady = false
            throw tagged(.disabled, "physical-observer-unavailable")
        }
        try fence.check()
    }
    func stop() {
        fence.observerReady = false
        if let source { CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes) }
        if let tap { CFMachPortInvalidate(tap) }
        source = nil; tap = nil
    }
}

// Query physical state transiently. Do not retain or report human key identities.
func foreignInputHeld() -> Bool {
    (0..<128).contains { CGEventSource.keyState(.combinedSessionState, key: CGKeyCode($0)) } ||
    [CGMouseButton.left, .right, .center].contains { CGEventSource.buttonState(.combinedSessionState, button: $0) }
}
