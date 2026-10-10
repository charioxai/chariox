import AppKit
import Darwin

@main struct PairedTargetsTests {
    @MainActor static func main() async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 20, width: 100, height: 100), scale: 2)
        var effects = 0
        var changeProtection = false
        weak var paired: PairedSession?
        let session = try PairedSession(fd: sockets[0], epoch: "test",
            scope: ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "ordinary"],
            surfaceProvider: { _ in surface }, execute: { _, source in
                if changeProtection {
                    _ = await paired!.reply(["id": 6, "epoch": "test", "method": "host.protect", "params": ["unknown": true, "values": [], "targets": []]])
                }
                try source.authority()
                effects += 1; return ["dispatched"]
            })
        paired = session
        _ = await session.reply(["id": 1, "epoch": "test", "method": "host.computer", "params": ["op": "start"]])
        _ = await session.reply(["id": 2, "epoch": "test", "method": "host.protect", "params": ["unknown": false, "values": ["vault-canary"], "targets": []]])
        for op in ["snapshot", "ocr", "target_action", "input"] {
            let response = await session.reply(["id": 3, "epoch": "test", "method": "host.computer", "params": [
                "op": op, "surface_id": "macos-seat", "generation": "test", "observed_by": "agent:fixture",
                "tree_revision": UInt64(1), "target_id": "opaque-ref", "action": "click", "input": ["kind": "text", "text": "public"]]])
            let error = response.reply["error"] as? [String: Any]
            precondition(response.reply["ok"] as? Bool == false && error?["reason"] as? String == "native-vault-provenance-unknown",
                "existing paired snapshot/OCR/action route bypassed native Vault policy")
        }
        precondition(effects == 0)
        _ = await session.reply(["id": 4, "epoch": "test", "method": "host.protect", "params": ["unknown": false, "values": [], "targets": []]])
        let valid = await session.reply(["id": 5, "epoch": "test", "method": "host.computer", "params": ["op": "input", "surface_id": "macos-seat", "generation": "test", "input": ["kind": "text", "text": "public"]]])
        precondition(valid.reply["ok"] as? Bool == true && effects == 1, "no-effect Vault refusal poisoned fresh admitted input")
        changeProtection = true
        let changed = await session.reply(["id": 7, "epoch": "test", "method": "host.computer", "params": ["op": "input", "surface_id": "macos-seat", "generation": "test", "input": ["kind": "text", "text": "public"]]])
        precondition(changed.reply["ok"] as? Bool == false && effects == 1, "protection changed after admission but native mutation was still allowed")
        print("PASS existing paired snapshot/OCR/target-action/input requests enforce native Vault policy before OS reads or effects; no-effect refusal preserves later admitted input")
    }
}
