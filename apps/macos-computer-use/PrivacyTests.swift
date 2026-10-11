import Foundation

@main struct PrivacyTests {
    static func main() throws {
        let clean = try NativePrivacyPolicy(["unknown": false, "values": [], "targets": []])
        precondition(clean.pixelsAllowed && clean.observationsAllowed)
        let vault = try NativePrivacyPolicy(["unknown": false, "values": ["vault-canary"], "targets": []])
        precondition(!vault.pixelsAllowed && !vault.observationsAllowed)
        let safe = try vault.scrub("public vault-canary label")
        precondition(safe == "public [redacted] label")
        let target = try NativePrivacyPolicy(["unknown": false, "values": [], "targets": [["kind": "native", "target": ["focus_window": 42]]]])
        precondition(!target.pixelsAllowed && !target.observationsAllowed)
        let unknown = try NativePrivacyPolicy(["unknown": true, "values": [], "targets": []])
        precondition(!unknown.pixelsAllowed)
        for policy: [String: Any] in [["unknown": false], ["unknown": false, "values": "malformed", "targets": []],
             ["unknown": false, "values": [7], "targets": []], ["unknown": false, "values": [], "targets": [7]],
             ["unknown": false, "values": [""], "targets": []]] {
            do { _ = try NativePrivacyPolicy(policy); preconditionFailure("malformed policy became public") } catch {}
        }
        print("PASS typed Vault/retired-value policy scrubs text and withholds native observations/pixels without proven field mapping; malformed provenance refuses")
    }
}
