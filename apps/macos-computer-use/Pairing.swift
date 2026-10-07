import Foundation
import Security

// macOS M1: kernel-paired identity/no-op mode. The helper never listens: it dials
// the kernel's 0700 rendezvous socket, verifies that peer's audit token and serves
// only that kernel. No ScreenCaptureKit, AX, CGEvent or permission API runs here.

/// One kernel request. Returns the reply and whether the helper must exit.
func pairedReply(_ request: [String: Any], epoch: String) -> (reply: [String: Any], stop: Bool) {
    var reply: [String: Any] = ["id": request["id"] ?? NSNull(), "epoch": epoch, "ok": true]
    let params = request["params"] as? [String: Any] ?? [:]
    func refuse(_ code: String) -> ([String: Any], Bool) {
        reply["ok"] = false
        reply["error"] = ["code": code]
        return (reply, false)
    }
    guard request["epoch"] as? String == epoch else { return refuse("user_domain_stale_epoch") }
    switch (request["method"] as? String, params["op"] as? String) {
    case ("heartbeat", _), ("host.protect", _): reply["result"] = [String: Any]()
    // Identity mode never holds input, so reset has nothing to release.
    case ("host.computer.reset", _): reply["result"] = ["released": true]
    case ("stop", _):
        reply["result"] = ["stopped": true]
        return (reply, true)
    case ("host.computer", "start"?), ("host.computer", "state"?):
        reply["result"] = ["surface_id": "macos-seat", "generation": epoch, "mode": "identity",
                           "capture": "unavailable", "input": "unavailable", "pid": Int(getpid())]
    default: return refuse("unsupported")
    }
    return (reply, false)
}

/// M5 pins the signed kernel. Until then only a drill build allowlists one kernel.
func kernelRequirement(_ info: [String: Any]) throws -> String {
    guard let hash = info["CharioxDrillKernelCDHash"] as? String, hash.count == 40,
          hash.allSatisfy(\.isHexDigit) else { throw Refusal.disabled }
    return "cdhash H\"\(hash.lowercased())\""
}

func peerSatisfies(_ token: audit_token_t, _ requirement: String) -> Bool {
    var token = token, code: SecCode?, rule: SecRequirement?
    let attributes = [kSecGuestAttributeAudit as String:
                        Data(bytes: &token, count: MemoryLayout<audit_token_t>.size)] as CFDictionary
    guard SecCodeCopyGuestWithAttributes(nil, attributes, [], &code) == errSecSuccess, let code,
          SecRequirementCreateWithString(requirement as CFString, [], &rule) == errSecSuccess,
          let rule else { return false }
    return SecCodeCheckValidity(code, [], rule) == errSecSuccess
}

func runPairing(_ path: String) throws -> Never {
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
    var buffer = Data()
    while let line = readLine(fd, &buffer),
          let request = try? JSONSerialization.jsonObject(with: line) as? [String: Any] {
        let (reply, stop) = pairedReply(request, epoch: epoch)
        try send(fd, reply)
        if stop { break }
    }
    // Stop, lease loss or kernel exit: identity mode has nothing else to fence.
    exit(0)
}

func connectUnix(_ path: String) throws -> Int32 {
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else { throw Refusal.arguments }
    withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    let connected = withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
        }
    }
    guard fd >= 0, connected == 0 else { throw Refusal.arguments }
    return fd
}

func send(_ fd: Int32, _ object: [String: Any]) throws {
    var data = try JSONSerialization.data(withJSONObject: object)
    data.append(0x0a)
    try data.withUnsafeBytes { raw in
        var offset = 0
        while offset < raw.count {
            let written = write(fd, raw.baseAddress! + offset, raw.count - offset)
            guard written > 0 else { throw Refusal.native }
            offset += written
        }
    }
}

func readLine(_ fd: Int32, _ buffer: inout Data) -> Data? {
    while true {
        if let newline = buffer.firstIndex(of: 0x0a) {
            let line = Data(buffer[..<newline])
            buffer.removeSubrange(...newline)
            return line
        }
        var chunk = [UInt8](repeating: 0, count: 4096)
        let count = read(fd, &chunk, chunk.count)
        guard count > 0, buffer.count < 1 << 20 else { return nil }
        buffer.append(contentsOf: chunk[..<count])
    }
}
