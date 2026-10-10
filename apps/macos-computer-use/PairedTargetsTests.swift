import AppKit
import Darwin

@main struct PairedTargetsTests {
    @MainActor static func main() async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 20, width: 100, height: 100), scale: 2)
        let session = try PairedSession(fd: sockets[0], epoch: "test",
            scope: ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "ordinary"],
            surfaceProvider: { _ in surface })
        _ = await session.reply(["id": 1, "epoch": "test", "method": "host.computer", "params": ["op": "start"]])
        _ = await session.reply(["id": 2, "epoch": "test", "method": "host.protect", "params": ["unknown": false, "values": ["vault-canary"], "targets": []]])
        for op in ["snapshot", "ocr", "target_action"] {
            let response = await session.reply(["id": 3, "epoch": "test", "method": "host.computer", "params": [
                "op": op, "surface_id": "macos-seat", "generation": "test", "observed_by": "agent:fixture",
                "tree_revision": UInt64(1), "target_id": "opaque-ref", "action": "click"]])
            let error = response.reply["error"] as? [String: Any]
            precondition(response.reply["ok"] as? Bool == false && error?["reason"] as? String == "native-vault-provenance-unknown",
                "existing paired snapshot/OCR/action route bypassed native Vault policy")
        }
        print("PASS existing paired snapshot/OCR/target-action requests enforce native Vault policy before OS reads or effects")
    }
}
