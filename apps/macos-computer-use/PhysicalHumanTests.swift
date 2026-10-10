import AppKit

@main struct PhysicalHumanTests {
    @MainActor static func main() throws {
        let fence = PhysicalHumanFence(marker: 123, pid: 42)
        fence.observerReady = true
        try fence.check()
        let own = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved,
                          mouseCursorPosition: CGPoint(x: 10, y: 20), mouseButton: .left)!
        fence.tag(own)
        fence.observe(.mouseMoved, event: own)
        try fence.check()
        // A marker alone does not exempt a foreign process or an unexpected event.
        own.setIntegerValueField(.eventSourceUnixProcessID, value: 99)
        fence.observe(.mouseMoved, event: own)
        do { try fence.check(); preconditionFailure("foreign input did not pause") }
        catch { precondition(refusalMessage(error) == "refused: disabled (physical-human-paused)") }
        fence.observerReady = true
        do { try fence.check(); preconditionFailure("observer recovery resumed automatically") }
        catch { }
        do { try fence.release(foreignHeld: true); preconditionFailure("foreign hold released the seat") }
        catch { }
        try fence.release(foreignHeld: false)
        try fence.check()
        fence.observe(.tapDisabledByTimeout, event: own)
        do { try fence.release(foreignHeld: false); preconditionFailure("disabled tap permitted release") }
        catch { }
        print("PASS physical human wins; marker/PID classification; pause survives observer recovery; explicit safe release only")
    }
}
