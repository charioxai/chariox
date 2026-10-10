import AppKit

@main struct NativeInputTests {
    @MainActor static func main() async throws {
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 50, width: 200, height: 100), scale: 2)
        let plan = try nativeInputPlan(["kind": "click", "x": UInt32(20), "y": UInt32(10), "button": 3], surface: surface)
        precondition(plan.point == CGPoint(x: -90, y: 55))
        precondition(plan.events.map(\.type) == [.otherMouseDown, .otherMouseUp])
        var posted: [CGEventType] = []
        let seat = NativeInputSeat()
        var safe = true
        do {
            try await seat.dispatch(plan, fence: { if !safe { throw Refusal.disabled } },
                releaseFence: { throw Refusal.permission }, post: { event in posted.append(event.type); safe = false })
            preconditionFailure("unsafe owned release accepted")
        } catch { precondition(refusalKind(error) == .ownedInput) }
        precondition(posted == [.otherMouseDown])
        do {
            try await seat.dispatch(plan, fence: {}, releaseFence: {}, post: { posted.append($0.type) })
            preconditionFailure("unresolved press accepted another action")
        } catch { precondition(refusalKind(error) == .ownedInput) }
        let clean = NativeInputSeat()
        var cancelled = false
        posted = []
        do {
            try await clean.dispatch(plan, fence: { if cancelled { throw Refusal.disabled } },
                releaseFence: {}, post: { posted.append($0.type); cancelled = true })
            preconditionFailure("cancelled input completed")
        } catch { precondition(refusalKind(error) == .disabled) }
        precondition(posted == [.otherMouseDown, .otherMouseUp])
        // Cleanup completed, so a subsequent explicitly admitted Action can run.
        try await clean.dispatch(plan, fence: {}, releaseFence: {}, post: { _ in })
        for bad: [String: Any] in [["kind":"keycode","keycode":0,"state":"down"],
            ["kind":"hold","key":"Left","duration_ms":10000],
            ["kind":"click","x":400,"y":0,"button":1],
            ["kind":"key","key":"cmd+r"], ["kind":"scroll","x":1,"y":1,"steps":0]] {
            do { _ = try nativeInputPlan(bad, surface: surface); preconditionFailure("unsafe input accepted") }
            catch { }
        }
        print("PASS paired native coordinate conversion, owned release/cancel/reset fencing and bounded input refusals")
    }
}
