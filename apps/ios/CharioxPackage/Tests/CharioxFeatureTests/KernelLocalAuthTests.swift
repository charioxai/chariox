import Foundation
import Testing
@testable import CharioxFeature

@Test func localKernelTokenIsOnlySentToLoopback() throws {
    let environment = ["SIMCTL_CHILD_CHARIOX_KERNEL_LOCAL_AUTH_TOKEN": "test-token"]
    for host in ["localhost", "127.0.0.1", "[::1]"] {
        let endpoint = try #require(URL(string: "ws://\(host):43118/kernel"))
        let request = KernelLocalAuth.connectionRequest(to: endpoint, environment: environment)
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer test-token")
    }
    for address in [
        "wss://relay.chariox.com/kernel", "ws://192.168.1.2:43118/kernel",
        "ws://localhost.example.com/kernel", "ws://127.0.0.1.example.com/kernel",
        "https://localhost/kernel", "ws://user@localhost/kernel",
    ] {
        let endpoint = try #require(URL(string: address))
        let request = KernelLocalAuth.connectionRequest(to: endpoint, environment: environment)
        #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
    }
}

@Test func localKernelTokenFileIsRereadAndFallsBackWhenUnavailable() throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: file) }
    let endpoint = try #require(URL(string: "ws://127.0.0.1:43118/kernel"))
    let environment = [
        "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE": file.path,
        "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN": "fallback-token",
    ]
    try "first-token\n".write(to: file, atomically: true, encoding: .utf8)
    #expect(KernelLocalAuth.connectionRequest(to: endpoint, environment: environment)
        .value(forHTTPHeaderField: "Authorization") == "Bearer first-token")
    try "second-token\n".write(to: file, atomically: true, encoding: .utf8)
    #expect(KernelLocalAuth.connectionRequest(to: endpoint, environment: environment)
        .value(forHTTPHeaderField: "Authorization") == "Bearer second-token")
    try FileManager.default.removeItem(at: file)
    #expect(KernelLocalAuth.connectionRequest(to: endpoint, environment: environment)
        .value(forHTTPHeaderField: "Authorization") == "Bearer fallback-token")
}
