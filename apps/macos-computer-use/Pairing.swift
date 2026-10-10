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

// One detached read at a time owns this buffer; the main actor never accesses it.
// Keeping it across reads preserves coalesced requests on the pairing socket.
final class PairingReader: @unchecked Sendable {
    private let fd: Int32
    private var buffer = Data()
    private let lock = NSLock()
    init(fd: Int32) { self.fd = fd }
    private func read() -> Data? {
        lock.lock(); defer { lock.unlock() }
        return readLine(fd, &buffer)
    }
    func next() async -> Data? { await Task.detached { [self] in read() }.value }
}
