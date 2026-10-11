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
        try await finalValidationCancellation(surface: surface)
        print("PASS existing paired snapshot/OCR/target-action/input requests enforce native Vault policy before OS reads or effects; no-effect refusal preserves later admitted input")
    }
    @MainActor static func finalValidationCancellation(surface: NativeSurface) async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        var buttonEffects = 0
        var finalMetadataReads = 0
        let session = try PairedSession(fd: sockets[0], epoch: "cancel-final-read",
            scope: ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "ordinary"],
            surfaceProvider: { _ in surface }, execute: { _, source in
                let registry = TargetRegistry<Int>()
                let context = TargetContext(processStart: "42", stamp: CaptureStamp(generation: "g", surface: surface, protection: 1))
                let reader = TargetReader<Int>(secure: { _ in false }, metadata: { _ in
                    TargetMetadata(role: kAXButtonRole, subrole: nil, label: "Public", bounds: surface.bounds,
                                   enabled: true, focused: false, actions: [kAXPressAction])
                }, children: { _ in [] })
                let target = try registry.observe(root: 1, context: context, reader: reader, scrub: { $0 })[0]
                let consumed = try registry.consume(target.ref, context: context, validate: { _, _ in true })
                // Use the same consumed dispatch boundary as NativeTargets.press.
                // The kernel closes pairing during the final synchronous metadata read.
                try consumed.perform(authority: source.authority, validate: { _, _ in
                    try source.authority()
                    finalMetadataReads += 1
                    precondition(shutdown(sockets[1], SHUT_RDWR) == 0)
                }, mutation: { _ in buttonEffects += 1 })
                try source.authority()
                return ["dispatched"]
            })
        _ = await session.reply(["id": 1, "epoch": "cancel-final-read", "method": "host.computer", "params": ["op": "start"]])
        _ = await session.reply(["id": 2, "epoch": "cancel-final-read", "method": "host.protect", "params": ["unknown": false, "values": [], "targets": []]])
        let response = await session.reply(["id": 3, "epoch": "cancel-final-read", "method": "host.computer", "params": [
            "op": "input", "surface_id": "macos-seat", "generation": "cancel-final-read", "input": ["kind": "text", "text": "public"]]])
        let error = response.reply["error"] as? [String: Any]
        precondition(finalMetadataReads == 1 && response.reply["ok"] as? Bool == false && error?["reason"] as? String == "grant-revoked")
        precondition(buttonEffects == 0, "pairing closed during final metadata validation but AXPress still dispatched")
        print("PASS final metadata cancellation refuses through real pairing authority with zero button effects")
    }

}
