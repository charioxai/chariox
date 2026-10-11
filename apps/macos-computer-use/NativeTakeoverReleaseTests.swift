import AppKit

@MainActor final class ReleaseTestObserver: PhysicalInputObserver {
    let fence = PhysicalHumanFence(marker: 123, pid: 42)
    func start() throws { fence.observerReady = true }
    var secure = false
    func check() throws { try fence.check() }
    func checkHealth() throws {
        try fence.checkHealth()
        if secure { throw Refusal.secure }
    }
    func stop() { fence.observerReady = false }
}

@main struct NativeTakeoverReleaseTests {
    @MainActor static func main() async throws {
        for keyboard in [false, true] {
            for loss in ["movement", "tap", "secure", "permission", "foreign-held", "foreign-down"] {
                let observer = ReleaseTestObserver()
                try observer.start()
                let request = Request(pid: 42, window: 7, target: "text")
                let saved = ClickGeometry(windowBounds: CGRect(x: 0, y: 0, width: 100, height: 100),
                    elementBounds: CGRect(x: 10, y: 10, width: 50, height: 50))
                let destination = NativeDestinationFence<Int>(validateTarget: { _ in }, pointerPoint: { saved.location },
                    selectedWindow: { _ in 1 }, focusedElement: { _ in 2 }, bind: { _, _ in }, geometry: { _ in saved },
                    windowBounds: { _ in saved.windowBounds }, bounds: { _ in saved.windowBounds }, role: { _ in kAXTextAreaRole },
                    hitAt: { _ in 2 }, parent: { _ in nil }, equal: ==, occlusion: { _, _, _ in }, authority: {})
                let plan = try nativeInputPlan(keyboard ? ["kind":"key", "key":"left"] :
                    ["kind":"click", "x":UInt32(35), "y":UInt32(35)], surface: NativeSurface(bounds: saved.windowBounds, scale: 1))
                let release = NativeReleaseFence(destination: destination, request: request, element: 2, plan: plan,
                    geometry: saved, observer: observer, foreignHeld: { loss == "foreign-held" }, postingAllowed: { loss != "permission" })
                let seat = NativeInputSeat()
                var sent: [CGEventType] = []
                do {
                    try await seat.dispatch(plan, fence: { try observer.check() }, releaseFence: { try release.check() }, post: { event in
                        sent.append(event.type)
                        if sent.count == 1 {
                            observer.fence.observe(.mouseMoved, event: CGEvent(source: nil)!)
                            if loss == "tap" { observer.fence.observe(.tapDisabledByTimeout, event: CGEvent(source: nil)!) }
                            if loss == "secure" { observer.secure = true }
                            if loss == "foreign-down" { observer.fence.observe(.keyDown, event: CGEvent(source: nil)!) }
                        }
                    })
                    preconditionFailure("physical takeover did not cancel Action")
                } catch {
                    precondition(refusalKind(error) == (loss == "movement" ? .disabled : .ownedInput), "physical movement blocked safe owned up: \(refusalMessage(error))")
                }
                if loss != "movement" {
                    precondition(sent.count == 1 && seat.unresolved, "unsafe owned up after \(loss)")
                    continue
                }
                // A source state table contains the owned down until cleanup posts up.
                // Hardware state and unrelated session controls must still fence it.
                for held in ["owned-session", "owned-hardware", "foreign-session"] {
                    let releaseEvent = plan.events.last!
                    let result = foreignInputHeld(excludingOwned: releaseEvent, keyState: { state, key in
                        keyboard && ((held == "owned-session" && state == .combinedSessionState && key == 123) ||
                            (held == "owned-hardware" && state == .hidSystemState && key == 123) ||
                            (held == "foreign-session" && state == .combinedSessionState && key == 124))
                    }, buttonState: { state, button in
                        !keyboard && ((held == "owned-session" && state == .combinedSessionState && button == .left) ||
                            (held == "owned-hardware" && state == .hidSystemState && button == .left) ||
                            (held == "foreign-session" && state == .combinedSessionState && button == .right))
                    })
                    precondition(result == (held != "owned-session"), "foreign hold classification: \(held)")
                }
                precondition(sent == plan.events.map(\.type) && !seat.unresolved, "owned press was not released")
                precondition(observer.fence.paused, "cleanup resumed admission")
                do { try observer.check(); preconditionFailure("paused observer admitted new input") } catch { }
                try observer.fence.release(foreignHeld: false)
                try await seat.dispatch(plan, fence: { try observer.check() }, releaseFence: { try release.check() }, post: { _ in })
            }
        }
        print("PASS foreign movement after mouse/key down safely releases owned up; pause persists until explicit release; unhealthy observer, secure input, permission loss and foreign holds refuse")
    }
}
