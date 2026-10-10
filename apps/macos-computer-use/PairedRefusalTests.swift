import AppKit
import Darwin

@main struct PairedRefusalTests {
    @MainActor static func main() async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        let scope: [String: Any] = ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target":"ordinary"]
        var attempts = 0
        var denied = true
        let session = try PairedSession(fd: sockets[0], epoch: "first", scope: scope,
            surfaceProvider: { _ in NativeSurface(bounds: CGRect(x:0,y:0,width:100,height:100),scale:1) }, execute: { _, _ in
                attempts += 1
                if denied { throw Refusal.permission }
                return ["dispatched"]
            })
        func call(_ input: [String: Any]?) async -> [String: Any] {
            var params: [String: Any] = ["op":input == nil ? "start":"input","surface_id":"macos-seat","generation":"first"]
            if let input { params["input"] = input }
            return await session.reply(["id":1,"epoch":"first","method":"host.computer","params":params]).reply
        }
        let identityRefusal = pairedReply(["id":1,"epoch":"first","method":"host.computer",
            "params":["op":"input","input":["kind":"key","key":"Right"]]], epoch:"first").reply
        precondition(identityRefusal["input_released"] as? Bool == true, "identity-only rejection poisoned input")
        _ = await call(nil)
        for input in [["kind":"text","text":String(repeating:"x",count:21)], ["kind":"key","key":"cmd+r"]] {
            let refusal = await call(input)
            precondition(refusal["ok"] as? Bool == false && refusal["input_released"] as? Bool == true,
                         "no-effect rejection did not confirm safe input state")
        }
        precondition(attempts == 0, "invalid input reached native dispatch")
        let permission = await call(["kind":"text","text":"x"])
        precondition(permission["input_released"] as? Bool == true)
        denied = false
        let valid = await call(["kind":"click","x":UInt32(10),"y":UInt32(10),"button":1])
        precondition(valid["ok"] as? Bool == true && attempts == 2, "ordinary refusal poisoned later input")
        print("PASS paired oversized text, unsupported key and pre-dispatch permission refusal confirm released state; later input usable")
    }
}
