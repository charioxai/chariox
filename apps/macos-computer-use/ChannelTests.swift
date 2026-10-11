import AppKit
import Darwin

@main struct ChannelTests {
    @MainActor static func main() async throws {
        var sockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0)
        defer { close(sockets[0]); close(sockets[1]) }
        let session = try PairedSession(fd: sockets[0], epoch: "test", scope: nil)
        try session.checkAuthority()
        shutdown(sockets[1], SHUT_RDWR)
        var mutations = 0
        do {
            try MacSource(authority: { try session.checkAuthority() }).checkPermission(.click)
            mutations += 1
            preconditionFailure("revoked socket reached permission or mutation")
        } catch { precondition(refusalMessage(error) == "refused: disabled (grant-revoked)") }
        precondition(mutations == 0)
        print("PASS paired socket EOF fences before native permission/mutation; reason grant-revoked")
        let scope = try pairedScope(["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "text"])!
        let surface = NativeSurface(bounds: CGRect(x: -200, y: -100, width: 300, height: 200), scale: 2)
        let click = try pairedInput(["kind": "click", "x": UInt32(100), "y": UInt32(50), "button": 1], scope: scope, surface: surface)
        precondition(click.target == "text" && click.operations == [.click])
        let path = try inputPath(.click, role: kAXTextAreaRole)
        precondition(path == .axTextClick)
        precondition(click.clickPoint == CGPoint(x: -150, y: -75))
        for input: [String: Any] in [["kind": "click", "x": UInt32(600), "y": UInt32(0), "button": 1],
            ["kind": "keycode", "keycode": 42, "state": "down"], ["kind": "text", "text": String(repeating: "x", count: 21)]] {
            do { _ = try pairedInput(input, scope: scope, surface: surface); preconditionFailure("unsupported input admitted") }
            catch { precondition([Refusal.target, .arguments, .text].contains(refusalKind(error))) }
        }
        var displaySockets: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &displaySockets) == 0)
        defer { close(displaySockets[0]); close(displaySockets[1]) }
        let display = try PairedSession(fd: displaySockets[0], epoch: "display",
            scope: ["pid": Int32(42), "window": UInt32(7), "owner_window": false, "target": "text"],
            surfaceProvider: { _ in surface })
        let started = await display.reply(["id": 1, "epoch": "display", "method": "host.computer", "params": ["op": "start"]])
        precondition(started.reply["ok"] as? Bool == true)
        let subscribed = await display.reply(["id": 2, "epoch": "display", "method": "host.display",
            "params": ["op": "subscribe", "surface_id": "macos-seat", "generation": "display",
                       "_attachment_owner": "viewer", "codecs": ["avc1.420033", "png"], "bitrate": 2_000_000]])
        precondition(subscribed.reply["ok"] as? Bool == true)
        precondition((subscribed.reply["result"] as? [String: Any])?["codec"] as? String == "avc1.420033", "private subscription must report the negotiated video codec")
        print("PASS paired display forwards and reports the shared AVC/PNG negotiation")
        let identityOnly = try pairedScope(nil); precondition(identityOnly == nil)
        do { _ = try pairedScope(["pid": Int32(42), "window": UInt32(7), "owner_window": true, "target": "text"]); preconditionFailure() }
        catch { precondition(refusalKind(error) == .arguments) }
        print("PASS paired scope/input bounds, negative-origin HiDPI mapping, and unsupported physical input")
    }
}
