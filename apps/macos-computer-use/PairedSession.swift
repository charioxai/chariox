import AppKit
import Darwin

// Explicit drill scope comes only from the authenticated kernel bootstrap.
// Unconfigured seats retain M1 identity mode. No agent can select another app.
@MainActor final class PairedSession {
    let fd: Int32
    let epoch: String
    let scope: Request?
    private var lastKernelRequest = ProcessInfo.processInfo.systemUptime
    private var revoked = false
    private var surface: NativeSurface?
    private let capture = PairedCapture()
    private var unresolvedInput = false
    private let readSurface: (Request) throws -> NativeSurface
    private let execute: (Request, MacSource) async throws -> [String]
    init(fd: Int32, epoch: String, scope: Any?,
         surfaceProvider: @escaping (Request) throws -> NativeSurface = nativeSurface,
         execute: @escaping (Request, MacSource) async throws -> [String] = { try await run($0, source: $1) }) throws {
        self.readSurface = surfaceProvider; self.execute = execute
        self.fd = fd; self.epoch = epoch
        self.scope = try pairedScope(scope)
    }
    func renew() { lastKernelRequest = ProcessInfo.processInfo.systemUptime }
    func checkAuthority() throws {
        guard !revoked else { throw tagged(.disabled, "grant-revoked") }
        if let scope, let surface {
            do {
                guard try readSurface(scope) == surface else { throw Refusal.target }
            } catch {
                revoked = true; capture.clear(); throw tagged(.target, "surface-geometry-changed")
            }
        }
        var byte: UInt8 = 0
        let count = recv(fd, &byte, 1, MSG_PEEK | MSG_DONTWAIT)
        guard count != 0, count >= 0 || errno == EAGAIN || errno == EWOULDBLOCK,
              ProcessInfo.processInfo.systemUptime - lastKernelRequest < 2 else {
            revoked = true; capture.clear()
            throw tagged(.disabled, "grant-revoked")
        }
    }
    func reply(_ request: [String: Any]) async -> (reply: [String: Any], stop: Bool) {
        guard let scope, request["epoch"] as? String == epoch else {
            return pairedReply(request, epoch: epoch)
        }
        let method = request["method"] as? String
        let params = request["params"] as? [String: Any] ?? [:]
        if method == "stop" { revoked = true; capture.clear(); return pairedReply(request, epoch: epoch) }
        if unresolvedInput, method != "stop" && method != "heartbeat" {
            return (["id": request["id"] ?? NSNull(), "epoch": epoch, "ok": false,
                     "error": ["code": "owner_reset_required"]], false)
        }
        if method == "heartbeat" {
            do {
                try checkAuthority()
                try capture.checkHealth(authority: { try self.checkAuthority() })
                return pairedReply(request, epoch: epoch)
            } catch {
                revoked = true; capture.clear()
                return (["id": request["id"] ?? NSNull(), "epoch": epoch, "ok": false,
                         "error": ["code": "browser_action_cancelled"]], true)
            }
        }
        if method == "host.protect" { capture.protect(params); return pairedReply(request, epoch: epoch) }
        if method == "host.computer", ["start", "state"].contains(params["op"] as? String ?? "") {
            do {
                if surface == nil { surface = try readSurface(scope) }
                try checkAuthority()
                var result = pairedReply(request, epoch: epoch)
                result.reply["result"] = ["surface_id": "macos-seat", "generation": epoch,
                    "mode": "scoped-drill", "capture": scope.ownerWindow ? "unavailable" : "protected-fixture-png", "input": "bounded-ax",
                    "width": surface!.width, "height": surface!.height, "placement": "host",
                    "pid": Int(getpid())]
                return result
            } catch {
                return (["id": request["id"] ?? NSNull(), "epoch": epoch, "ok": false,
                         "error": ["code": "user_domain_stale_epoch"]], true)
            }
        }
        if method == "host.display" {
            var reply: [String: Any] = ["id": request["id"] ?? NSNull(), "epoch": epoch, "ok": false]
            do {
                try checkAuthority()
                guard !scope.ownerWindow, let surface,
                      params["surface_id"] as? String == "macos-seat", params["generation"] as? String == epoch,
                      let owner = params["_attachment_owner"] as? String else { throw tagged(.arguments, "invalid-capture-binding") }
                switch params["op"] as? String {
                case "subscribe":
                    let id = try capture.attachment.subscribe(owner: owner, generation: epoch)
                    reply["result"] = ["subscription_id": id, "surface_id": "macos-seat", "generation": epoch,
                        "display_kind": "native", "width": surface.width, "height": surface.height, "device_scale_factor": surface.scale, "codec": "png"]
                case "attach":
                    guard let id = params["subscription_id"] as? String else { throw Refusal.arguments }
                    try capture.attachment.attach(id: id, owner: owner, generation: epoch)
                    reply["result"] = ["display_attached": true]
                case "sample":
                    guard let id = params["subscription_id"] as? String else { throw Refusal.arguments }
                    let frame = try await capture.attachment.sample(id: id, owner: owner,
                        after: params["after_sequence"] as? UInt64, repair: params["repair"] as? Bool == true,
                        authority: { try self.checkAuthority() }, capture: {
                            try await self.capture.raster(scope, surface: surface, generation: self.epoch, useStream: true,
                                                          authority: { try self.checkAuthority() })
                        })
                    reply["result"] = ["frame_sent": frame != nil, "display_frame": frame as Any? ?? NSNull()]
                case "close":
                    guard let id = params["subscription_id"] as? String else { throw Refusal.arguments }
                    capture.attachment.close(id: id, owner: owner); reply["result"] = ["closed": true]
                default: throw Refusal.arguments
                }
                try checkAuthority(); reply["ok"] = true
            } catch {
                reply["error"] = ["code": unresolvedInput ? "owner_reset_required" : (revoked ? "browser_action_cancelled" : "unsupported"),
                                  "reason": (error as? RefusalDetail)?.reason ?? "native-refused"]
            }
            return (reply, revoked)
        }
        guard method == "host.computer", ["input", "screenshot"].contains(params["op"] as? String ?? "") else {
            return pairedReply(request, epoch: epoch)
        }
        var reply: [String: Any] = ["id": request["id"] ?? NSNull(), "epoch": epoch, "ok": false]
        do {
            try checkAuthority()
            guard !unresolvedInput else { throw tagged(.ownedInput, "owned-release-failed") }
            guard params["surface_id"] as? String == "macos-seat",
                  params["generation"] as? String == epoch else { throw tagged(.target, "stale-native-scope") }
            guard let surface else { throw tagged(.disabled, "scope-not-started") }
            if params["op"] as? String == "screenshot" {
                let image = try await capture.screenshot(scope, surface: surface, generation: epoch,
                                                         authority: { try self.checkAuthority() })
                try checkAuthority(); reply["ok"] = true; reply["result"] = image
                return (reply, false)
            }
            guard let input = params["input"] as? [String: Any] else { throw tagged(.arguments, "invalid-paired-input") }
            let operation = try pairedInput(input, scope: scope, surface: surface)
            let receipts = try await execute(operation, MacSource(authority: { try self.checkAuthority() }))
            try checkAuthority()
            reply["ok"] = true
            reply["result"] = ["dispatched": true, "receipts": receipts]
        } catch {
            if refusalKind(error) == .ownedInput { unresolvedInput = true }
            // Fixed public tags only. No text, AX tree or app errors cross the socket.
            reply["error"] = ["code": unresolvedInput ? "owner_reset_required" : (revoked ? "browser_action_cancelled" : "unsupported"),
                              "reason": (error as? RefusalDetail)?.reason ?? "native-refused"]
            reply["input_released"] = !unresolvedInput
        }
        return (reply, revoked)
    }
}

func pairedScope(_ value: Any?) throws -> Request? {
    guard let value, !(value is NSNull) else { return nil }
    guard let data = value as? [String: Any], Set(data.keys) == ["pid", "window", "owner_window", "target"],
          let pid = data["pid"] as? Int32, pid > 0, let window = data["window"] as? UInt32, window > 0,
          let owner = data["owner_window"] as? Bool, let target = data["target"] as? String,
          (owner ? target == "focused" : ["ordinary", "text", "button", "scroll"].contains(target)) else {
        throw tagged(.arguments, "invalid-paired-scope")
    }
    return Request(enabled: true, ownerWindow: owner, pid: pid, window: window, target: target)
}

// Only the already-bounded M0 paths are enabled for revocation drills. M3 owns
// physical holds, persistent key state and local-human observation.
func pairedInput(_ input: [String: Any], scope: Request, surface: NativeSurface) throws -> Request {
    var operation = scope
    switch input["kind"] as? String {
    case "text":
        guard let text = input["text"] as? String else { throw tagged(.arguments, "invalid-paired-text") }
        _ = try textUnits(text); operation.operations = [.text(text)]
    case "click":
        guard let x = input["x"] as? UInt32, let y = input["y"] as? UInt32,
              input["button"] as? Int == 1 else { throw tagged(.arguments, "invalid-paired-click") }
        operation.clickPoint = try surface.point(x: x, y: y); operation.operations = [.click]
    default: throw tagged(.arguments, "unsupported-paired-input")
    }
    return operation
}

@MainActor func runPairing(_ path: String) async throws -> Never {
    var directory = stat()
    guard lstat(path, &directory) == 0, directory.st_uid == getuid(),
          directory.st_mode & S_IFMT == S_IFDIR, directory.st_mode & 0o077 == 0 else { throw Refusal.arguments }
    // Read, then delete, the one-use bootstrap before connecting.
    let bootstrapPath = path + "/bootstrap"
    let data = try Data(contentsOf: URL(fileURLWithPath: bootstrapPath))
    unlink(bootstrapPath)
    guard let bootstrap = try JSONSerialization.jsonObject(with: data) as? [String: Any],
          let token = bootstrap["token"] as? String, let epoch = bootstrap["epoch"] as? String,
          let kernelPID = bootstrap["kernel_pid"] as? Int else { throw Refusal.arguments }
    let requirement = try kernelRequirement(Bundle.main.infoDictionary ?? [:])
    signal(SIGPIPE, SIG_IGN)
    let fd = try connectUnix(path + "/s")
    // Admit only the expected kernel process, by kernel-reported identity.
    var peer = audit_token_t(), length = socklen_t(MemoryLayout<audit_token_t>.size)
    guard getsockopt(fd, 0 /* SOL_LOCAL */, 6 /* LOCAL_PEERTOKEN */, &peer, &length) == 0,
          peer.val.1 == getuid(), Int(peer.val.5) == kernelPID,
          peerSatisfies(peer, requirement) else { throw Refusal.target }
    // The kernel heartbeats every second; two silent seconds end the lease.
    var lease = timeval(tv_sec: 2, tv_usec: 0)
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &lease, socklen_t(MemoryLayout<timeval>.size))
    try send(fd, ["token": token, "epoch": epoch, "pid": Int(getpid())])
    let session = try PairedSession(fd: fd, epoch: epoch, scope: bootstrap["scope"])
    var buffer = Data()
    while let line = readLine(fd, &buffer),
          let request = try? JSONSerialization.jsonObject(with: line) as? [String: Any] {
        session.renew()
        let (reply, stop) = await session.reply(request)
        try send(fd, reply)
        if stop { break }
    }
    // Stop, lease loss or kernel exit: identity mode has nothing else to fence.
    exit(0)
}

