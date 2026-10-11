import Foundation

// This is the existing kernel Vault registry, including retired scrub-only
// values. No app category, arbitrary ordinary text or OCR heuristic adds masks.
// Its Linux/browser refs cannot establish a Mac field's geometry. Withhold
// rather than treating those refs as AX IDs or broadening masking scope.
struct NativePrivacyPolicy {
    private let values: [String]
    private let unknown: Bool
    private let hasTargets: Bool
    init(_ policy: [String: Any]) throws {
        guard Set(policy.keys) == ["unknown", "values", "targets"],
              let unknown = policy["unknown"] as? Bool,
              let values = policy["values"] as? [String], values.count <= 1024,
              values.allSatisfy({ !$0.isEmpty && $0.utf8.count <= 4096 }),
              values.reduce(0, { $0 + $1.utf8.count }) <= 1024 * 1024,
              let targets = policy["targets"] as? [[String: Any]], targets.count <= 256 else {
            throw tagged(.secure, "native-vault-provenance-unknown")
        }
        self.unknown = unknown; self.values = values.sorted { $0.count > $1.count }; hasTargets = !targets.isEmpty
    }
    var pixelsAllowed: Bool { !unknown && values.isEmpty && !hasTargets }
    var observationsAllowed: Bool { pixelsAllowed }
    func scrub(_ text: String) throws -> String {
        guard !unknown, text.utf8.count <= 4096 else { throw tagged(.secure, "native-vault-provenance-unknown") }
        return values.reduce(text) { $0.replacingOccurrences(of: $1, with: "[redacted]") }
    }
}
