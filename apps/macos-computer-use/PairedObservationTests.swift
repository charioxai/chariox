import AppKit

@main struct PairedObservationTests {
    @MainActor static func main() async throws {
        let capture = PairedCapture()
        let scope = Request(enabled: true, ownerWindow: false, pid: 42, window: 7, target: "ordinary")
        let surface = NativeSurface(bounds: CGRect(x: 0, y: 0, width: 100, height: 100), scale: 1)
        for policy: [String: Any] in [["unknown": false, "values": ["vault-canary"], "targets": []],
            ["unknown": false, "values": [], "targets": [["kind": "native", "target": ["focus_window": 42]]]],
            ["unknown": false, "values": "malformed", "targets": []], ["unknown": true, "values": [], "targets": []]] {
            capture.protect(policy)
            do {
                _ = try await capture.observe(scope, surface: surface, generation: "test", authority: {})
                preconditionFailure("unmapped Vault/unknown policy published native targets or OCR")
            } catch { precondition((error as? RefusalDetail)?.reason == "native-vault-provenance-unknown") }
        }
        capture.clear()
        do { _ = try await capture.observe(scope, surface: surface, generation: "test", authority: {}); preconditionFailure() }
        catch { precondition(refusalKind(error) == .secure) }
        print("PASS actual capture/target/OCR integration refuses unmapped Vault and malformed/unknown policy before OS reads; Stop removes observation authority")
    }
}
