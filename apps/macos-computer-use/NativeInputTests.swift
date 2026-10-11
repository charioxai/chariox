import AppKit

@MainActor final class ReleaseObserverForDestinationTests: PhysicalInputObserver {
    let fence = PhysicalHumanFence()
    init() { fence.observerReady = true }
    func start() throws { fence.observerReady = true }
    func check() throws { try fence.check() }
    func stop() { fence.observerReady = false }
}

@main struct NativeInputTests {
    @MainActor static func main() async throws {
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 50, width: 200, height: 100), scale: 2)
        let plan = try nativeInputPlan(["kind": "click", "x": UInt32(20), "y": UInt32(10), "button": 2], surface: surface)
        precondition(plan.point == CGPoint(x: -90, y: 55))
        precondition(plan.events.map(\.type) == [.otherMouseDown, .otherMouseUp])
        for (number, down, up, button) in [(1, CGEventType.leftMouseDown, CGEventType.leftMouseUp, CGMouseButton.left),
            (2, .otherMouseDown, .otherMouseUp, .center), (3, .rightMouseDown, .rightMouseUp, .right)] {
            let click = try nativeInputPlan(["kind":"click", "x":UInt32(20), "y":UInt32(10), "button":number], surface: surface)
            precondition(click.events.map(\.type) == [down, up], "shared button \(number) changed meaning")
            precondition(click.events.allSatisfy { $0.getIntegerValueField(.mouseEventButtonNumber) == Int64(button.rawValue) })
        }
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
        // Inject AX/CG reads at the production destination fence. No native
        // event is posted, and the release fence itself is never replaced.
        for loss in ["focus", "cover", "same-process-cover", "geometry", "hit", "key-focus", "key-geometry", "authority"] {
            var changed = false
            let request = Request(pid: 42, window: 7, target: "text")
            let saved = ClickGeometry(windowBounds: CGRect(x: 0, y: 0, width: 100, height: 100),
                elementBounds: CGRect(x: 10, y: 10, width: 50, height: 50))
            let destination = NativeDestinationFence<Int>(validateTarget: { _ in }, pointerPoint: { saved.location },
                selectedWindow: { _ in
                    try checkWindowBinding(pid: 42, frontmostPID: changed && loss == "focus" ? 43 : 42,
                        axMatches: 1, cgMatches: 1, focused: true)
                    return 1
                }, focusedElement: { _ in changed && loss == "key-focus" ? 3 : 2 },
                bind: { _, _ in }, geometry: { _ in
                    ClickGeometry(windowBounds: saved.windowBounds,
                        elementBounds: changed && ["geometry", "key-geometry"].contains(loss) ? saved.elementBounds.offsetBy(dx: 1, dy: 0) : saved.elementBounds)
                }, windowBounds: { _ in saved.windowBounds }, bounds: { _ in saved.windowBounds },
                role: { _ in kAXTextAreaRole }, hitAt: { _ in changed && loss == "hit" ? 3 : 2 },
                parent: { _ in nil }, equal: ==, occlusion: { request, point, rect in
                    let target = WindowOwnership(id: 7, pid: 42, bounds: rect, alpha: 1)
                    let cover = WindowOwnership(id: 9, pid: loss == "same-process-cover" ? 42 : 43, bounds: rect, alpha: 1)
                    try checkWindowStack(changed && ["cover", "same-process-cover"].contains(loss) ? [cover, target] : [target],
                        request: request, point: point, windowBounds: rect)
                }, authority: { if changed { throw Refusal.disabled } })
            let keyboard = ["key-focus", "key-geometry"].contains(loss)
            let admitted = try nativeInputPlan(keyboard ? ["kind":"key", "key":"left"] :
                ["kind":"click", "x":UInt32(35), "y":UInt32(35)],
                surface: NativeSurface(bounds: saved.windowBounds, scale: 1))
            let release = NativeReleaseFence(destination: destination, request: request, element: 2,
                plan: admitted, geometry: saved, observer: ReleaseObserverForDestinationTests(), foreignHeld: { false }, postingAllowed: { true })
            let seat = NativeInputSeat()
            var sent: [CGEventType] = []
            do {
                try await seat.dispatch(admitted, fence: {
                    try destination.check(request, element: 2, typing: keyboard,
                        clickGeometry: keyboard ? nil : saved, eventLocation: admitted.point, physicalKey: keyboard)
                }, releaseFence: { try release.check() }, post: { sent.append($0.type); changed = true })
                preconditionFailure("lost release destination admitted")
            } catch { precondition(refusalKind(error) == (loss == "authority" ? .disabled : .ownedInput), "\(loss): incorrect cleanup verdict") }
            if loss == "authority" {
                precondition(sent.count == 2 && !seat.unresolved, "safe cancelled Action could not release")
            } else {
                precondition(sent.count == 1 && seat.unresolved, "\(loss): release posted over lost destination")
            }
        }
        print("PASS production release fence refuses seven destination losses; safe Action cancellation releases")
        print("PASS paired native coordinate conversion, owned release/cancel/reset fencing and bounded input refusals")
    }
}
