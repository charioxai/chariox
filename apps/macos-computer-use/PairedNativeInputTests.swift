import AppKit
import Darwin

@MainActor final class TestInputObserver: PhysicalInputObserver {
    let fence = PhysicalHumanFence()
    func start() throws { fence.observerReady = true }
    func check() throws { try fence.check() }
    func stop() { fence.observerReady = false }
}

@main struct PairedNativeInputTests {
    @MainActor static func main() async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        let scope: [String: Any] = ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "button"]
        let surface = NativeSurface(bounds: CGRect(x: 0, y: 0, width: 100, height: 100), scale: 1)
        let observer = TestInputObserver()
        var posted: [CGEventType] = []
        let native = PairedNativeInput(observer: observer, foreignHeld: { false }, execute: { plan, _, _, observer, seat in
            try await seat.dispatch(plan, fence: { try observer.check() }, releaseFence: { try observer.check() }, post: { posted.append($0.type) })
            return ["dispatched"]
        })
        let session = try PairedSession(fd: sockets[0], epoch: "first", scope: scope,
            surfaceProvider: { _ in surface }, nativeInput: native)
        func call(_ params: [String: Any], method: String = "host.computer") async -> [String: Any] {
            let request = try! JSONSerialization.jsonObject(with: JSONSerialization.data(withJSONObject:
                ["id":1,"epoch":"first","method":method,"params":params])) as! [String: Any]
            return await session.reply(request).reply
        }
        let started = await call(["op":"start"]); precondition(started["ok"] as? Bool == true)
        _ = await call(["unknown": false, "values": [], "targets": []], method: "host.protect")
        let input: [String: Any] = ["op":"input","surface_id":"macos-seat","generation":"first",
            "input":["kind":"move","x":20,"y":20]]
        let admitted = await call(input); precondition(admitted["ok"] as? Bool == true)
        let foreign = CGEvent(source: nil)!
        observer.fence.observe(.mouseMoved, event: foreign)
        let paused = await call(input); precondition(paused["ok"] as? Bool == false)
        precondition(posted == [.mouseMoved], "paired input ignored physical human takeover")
        // Action failure cleanup must not silently hand control back to the agent.
        _ = await call([:], method: "host.computer.reset")
        let stillPaused = await call(input); precondition(stillPaused["ok"] as? Bool == false)
        let state = await call(["op":"state"])
        precondition((state["result"] as? [String: Any])?["physical_human"] as? Bool == true)
        _ = await call(["physical_human_release":true], method: "host.computer.reset")
        let resumed = await call(input); precondition(resumed["ok"] as? Bool == true)
        print("PASS paired physical input pauses before dispatch; ordinary reset cannot resume; explicit human release resumes")
    }
}
