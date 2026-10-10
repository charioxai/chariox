import AppKit
import Darwin

@main struct PairedFailureTests {
    @MainActor static func main() async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        let scope: [String: Any] = ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "text"]
        var surface = NativeSurface(bounds: CGRect(x: 0, y: 0, width: 100, height: 100), scale: 1)
        var nativeCalls = 0
        let session = try PairedSession(fd: sockets[0], epoch: "first", scope: scope,
            surfaceProvider: { _ in surface }, execute: { _, _ in
                nativeCalls += 1; throw tagged(.ownedInput, "owned-release-failed")
            })
        func request(_ method: String, _ params: [String: Any]) -> [String: Any] {
            ["id": 1, "epoch": "first", "method": method, "params": params]
        }
        let started = await session.reply(request("host.computer", ["op": "start"]))
        precondition(started.reply["ok"] as? Bool == true)
        let input = request("host.computer", ["op": "input", "surface_id": "macos-seat", "generation": "first",
            "input": ["kind": "text", "text": "x"]])
        let failed = await session.reply(input)
        precondition((failed.reply["error"] as? [String: Any])?["code"] as? String == "owner_reset_required")
        let reset = await session.reply(request("host.computer.reset", [:]))
        precondition(reset.reply["ok"] as? Bool == false && reset.reply["result"] == nil)
        _ = await session.reply(input)
        precondition(nativeCalls == 1, "unresolved owned input dispatched a new action")
        let fresh = try PairedSession(fd: sockets[0], epoch: "first", scope: scope,
                                      surfaceProvider: { _ in surface })
        _ = await fresh.reply(request("host.computer", ["op": "start"]))
        surface = NativeSurface(bounds: CGRect(x: 1, y: 0, width: 100, height: 100), scale: 1)
        let invalidated = await fresh.reply(request("host.computer", ["op": "state"]))
        precondition(invalidated.reply["ok"] as? Bool == false && invalidated.stop,
                     "invalid geometry did not end the pairing")
        print("PASS paired failed-release latch, reset/new-action refusal, and geometry invalidation ends pairing")
    }
}
