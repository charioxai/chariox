import Foundation
import Security

final class FakeSource: NativeSource {
    var calls = 0
    var typed: [UInt16] = []
    var secureInput = false
    var bundleIdentifier: String? = "com.apple.TextEdit"
    var appleSigned = true
    var screenRecording = true
    var accessibility = true
    var permissionChecks: [String] = []
    func checkPermission(_ operation: Operation) throws {
        try checkOperationPermissions(operation, screenCaptureAccess: {
            permissionChecks.append("screen")
            return screenRecording
        }, accessibilityAccess: {
            permissionChecks.append("accessibility")
            return accessibility
        })
    }
    func validateTarget(_ request: Request) throws {
        guard request.pid == 42, request.window == 1 else { throw Refusal.target }
        if request.ownerWindow {
            try checkOwnerTarget(bundleIdentifier: bundleIdentifier, pid: request.pid) { pid, requirement in
                precondition(pid == request.pid)
                precondition(requirement == "identifier \"com.apple.TextEdit\" and anchor apple")
                return appleSigned
            }
        }
        guard !secureInput else { throw Refusal.secure }
    }
    func perform(_ operation: Operation, request: Request) async throws -> String {
        calls += 1
        if case .text(let text) = operation { typed = try textUnits(text) }
        return "fake-frame"
    }
}
@main struct Tests {
    static func main() async throws {
        let revoked = FakeSource()
        revoked.screenRecording = false; revoked.accessibility = false
        do {
            _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                      operations: [.capture("unused")]), source: revoked)
            print("FAIL revoked capture admitted"); exit(1)
        } catch Refusal.permission { }
        guard revoked.permissionChecks == ["screen"], revoked.calls == 0 else {
            print("FAIL capture did not check Screen Recording before Accessibility"); exit(1)
        }
        print("PASS revoked capture checks Screen Recording before Accessibility")
        revoked.accessibility = true
        revoked.permissionChecks = []
        do {
            _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                      operations: [.capture("unused")]), source: revoked)
            print("FAIL Screen Recording revocation admitted capture"); exit(1)
        } catch Refusal.permission { }
        precondition(revoked.permissionChecks == ["screen"] && revoked.calls == 0)
        revoked.permissionChecks = []
        _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                  operations: [.text("public")]), source: revoked)
        precondition(revoked.permissionChecks == ["accessibility"] && revoked.calls == 1)
        precondition(revoked.typed == [112, 117, 98, 108, 105, 99])
        print("PASS Screen Recording revocation refuses frames but permits short text")
        revoked.accessibility = false
        revoked.permissionChecks = []
        do {
            _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                      operations: [.text("public")]), source: revoked)
            print("FAIL Accessibility revocation admitted input"); exit(1)
        } catch Refusal.permission { }
        precondition(revoked.permissionChecks == ["accessibility"] && revoked.calls == 1)
        print("PASS subsequent Accessibility revocation refuses input without a capture check")
        guard signingInformationFlags().rawValue & kSecCSSigningInformation != 0 else {
            print("FAIL signing information flag missing"); exit(1)
        }
        do {
            _ = try textUnits(String(repeating: "😀", count: 11))
            print("FAIL 22-unit emoji payload admitted"); exit(1)
        } catch Refusal.text { }
        var identityFailures = 0
        for (bundle, signed) in [("com.apple.Safari", true), ("ai.chariox.computer-fixture", true),
                                 ("com.apple.TextEdit", false)] {
            let refused = FakeSource()
            refused.bundleIdentifier = bundle; refused.appleSigned = signed
            do {
                _ = try await run(Request(enabled: true, ownerWindow: true, pid: 42, window: 1,
                                          target: "focused", operations: [.capture("unused"), .text("public")]), source: refused)
                print("FAIL owner target admitted bundle=\(bundle) appleSigned=\(signed)")
                identityFailures += 1
            } catch Refusal.target {
                precondition(refused.calls == 0 && refused.permissionChecks.isEmpty)
            }
        }
        if identityFailures > 0 { exit(1) }
        for bundle in [nil, "com.apple.Safari"] as [String?] {
            do {
                try checkOwnerTarget(bundleIdentifier: bundle, pid: 42) { _, _ in
                    preconditionFailure("non-allowlisted bundle reached signing check")
                }
                print("FAIL discovery identity admitted"); exit(1)
            } catch Refusal.target { }
        }
        // The test process is not Apple TextEdit, even if a caller supplies its bundle ID.
        do {
            try checkOwnerTarget(bundleIdentifier: "com.apple.TextEdit", pid: getpid())
            print("FAIL test process passed Apple TextEdit code requirement"); exit(1)
        } catch Refusal.target { }
        var signingChecks = 0
        try checkOwnerTarget(bundleIdentifier: "com.apple.TextEdit", pid: 42) { pid, requirement in
            precondition(pid == 42 && requirement == "identifier \"com.apple.TextEdit\" and anchor apple")
            signingChecks += 1
            return true
        }
        precondition(signingChecks == 1)
        print("PASS shared owner/discovery allowlist requires TextEdit bundle and running Apple code")
        print("PASS owner operations refuse other bundles and TextEdit impostors before dispatch")
        let source = FakeSource()
        do {
            let owner = try Request.parse(["--enable-owner-window", "--owner-pid", "42", "--window-id", "1", "--target", "focused", "--text", "public"])
            _ = try await run(owner, source: source)
        } catch { print("FAIL explicit owner-selected window refused"); exit(1) }
        print("PASS explicit owner-selected window scope")
        source.calls = 0
        for arguments in [
            ["--enable-owner-window", "--fixture-pid", "42"],
            ["--enable-fixture", "--owner-pid", "42"],
            ["--enable-owner-window", "--enable-fixture"],
            ["--enable-owner-window", "--owner-pid", "42", "--fixture-pid", "42"]
        ] {
            do { _ = try Request.parse(arguments); print("FAIL mixed scope admitted"); exit(1) }
            catch Refusal.arguments { }
        }
        for request in [Request(enabled: true, ownerWindow: true, pid: 42, window: 1, target: "ordinary", operations: [.read]),
                        Request(enabled: true, ownerWindow: true, pid: 42, target: "focused", operations: [.read])] {
            let isolated = FakeSource()
            do { _ = try await run(request, source: isolated); print("FAIL invalid owner window admitted"); exit(1) }
            catch Refusal.target { }
            precondition(isolated.calls == 0)
        }
        let secureOwner = FakeSource(); secureOwner.secureInput = true
        do {
            _ = try await run(Request(enabled: true, ownerWindow: true, pid: 42, window: 1,
                                      target: "focused", operations: [.capture("unused"), .text("public")]), source: secureOwner)
            print("FAIL secure input admitted in owner window"); exit(1)
        } catch Refusal.secure { }
        precondition(secureOwner.calls == 0)
        let longOwner = FakeSource()
        do {
            _ = try await run(Request(enabled: true, ownerWindow: true, pid: 42, window: 1,
                                      target: "focused", operations: [.capture("unused"), .text(String(repeating: "😀", count: 11))]), source: longOwner)
            print("FAIL overflowing owner batch admitted"); exit(1)
        } catch Refusal.text { }
        precondition(longOwner.calls == 0)
        print("PASS mixed scopes, secure input and invalid owner batches refuse before dispatch")
        try checkWindowBinding(pid: 42, frontmostPID: 42, axMatches: 1, cgMatches: 1, focused: true)
        for facts in [(43 as Int32?, 1, 1, true), (nil, 1, 1, true),
                      (42, 0, 1, true), (42, 2, 1, true),
                      (42, 1, 0, true), (42, 1, 2, true), (42, 1, 1, false)] {
            do {
                try checkWindowBinding(pid: 42, frontmostPID: facts.0,
                                       axMatches: facts.1, cgMatches: facts.2, focused: facts.3)
                print("FAIL unsafe window binding admitted"); exit(1)
            } catch Refusal.target { }
        }
        print("PASS frontmost, unique geometry and AX window focus fences")
        let twentyUnits = try textUnits(String(repeating: "😀", count: 10))
        precondition(twentyUnits == Array(String(repeating: "😀", count: 10).utf16))
        for text in [String(repeating: "a", count: 19) + "😀", String(repeating: "👩‍💻", count: 5), "a" + String(repeating: "\u{301}", count: 20)] {
            do { _ = try textUnits(text); print("FAIL grapheme boundary overflow admitted"); exit(1) }
            catch Refusal.text { }
        }
        print("PASS 20-unit Unicode cap refuses whole overflowing graphemes; signing flags include Team ID data")
        do {
            _ = try await run(Request(operations: [.capture("unused")]), source: source)
            print("FAIL disabled capture was admitted"); exit(1)
        } catch Refusal.disabled { }
        precondition(source.calls == 0, "disabled request reached native source")
        print("PASS disabled operations never reach native source")
        do {
            _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                      target: "Safari", operations: [.read]), source: source)
            print("FAIL non-fixture target admitted"); exit(1)
        } catch Refusal.target { }
        precondition(source.calls == 0)
        print("PASS non-fixture target refused")
        for operation in [Operation.read, .click, .scroll, .text("canary")] {
            do {
                _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                          target: "secure", operations: [operation]), source: source)
                print("FAIL secure target admitted"); exit(1)
            } catch Refusal.secure { }
        }
        precondition(source.calls == 0)
        print("PASS secure target refuses reads and input")
        for text in ["", "\n", "\t", "\u{1b}", "\u{2028}", String(repeating: "a", count: 21)] {
            do {
                _ = try await run(Request(enabled: true, pid: 42, window: 1,
                                          operations: [.text(text)]), source: source)
                print("FAIL unsupported text admitted"); exit(1)
            } catch Refusal.text { }
        }
        precondition(source.calls == 0)
        print("PASS controls and oversized text refused before native dispatch")
        let result = try await run(Request(enabled: true, pid: 42, window: 1,
                                          operations: [.text("é e\u{301} 😀 中")]), source: source)
        precondition(source.typed == [233, 32, 101, 769, 32, 55357, 56832, 32, 20013])
        precondition(result == ["fake-frame"])
        let idle = FakeSource()
        _ = try await run(Request(enabled: true, pid: 42, window: 1), source: idle)
        precondition(idle.calls == 0, "no operation flag must do nothing")
        print("PASS Unicode preserved and absent operation flags remain inert")
        let parsed = try Request.parse(["--enable-fixture", "--fixture-pid", "42", "--window-id", "1",
                                        "--capture", "unused", "--ax-read", "--click", "--scroll", "--text", "中"])
        precondition(parsed.operations == [.capture("unused"), .read, .click, .scroll, .text("中")])
        let allowed = FakeSource()
        let receipts = try await run(parsed, source: allowed)
        precondition(receipts == Array(repeating: "fake-frame", count: 5) && allowed.typed == [20013])
        for arguments in [["--unknown"], ["--text"], ["--identity", "--click"]] {
            do { _ = try Request.parse(arguments); print("FAIL invalid flags admitted"); exit(1) }
            catch Refusal.arguments { }
        }
        for request in [Request(enabled: true, operations: [.read]),
                        Request(enabled: true, pid: 42, window: 1, operations: [.capture("unused"), .text("\n")])] {
            let isolated = FakeSource()
            do { _ = try await run(request, source: isolated); print("FAIL invalid batch admitted"); exit(1) }
            catch is Refusal { }
            precondition(isolated.calls == 0)
        }
        print("PASS explicit operation flags and invalid batches")
        for secure in [false, true] {
            let unsafe = FakeSource(); unsafe.secureInput = secure
            do {
                _ = try await run(Request(enabled: true, pid: secure ? 42 : 43, window: 1,
                                          operations: [.capture("unused")]), source: unsafe)
                print("FAIL unsafe native binding reached capture"); exit(1)
            } catch is Refusal { }
            precondition(unsafe.calls == 0)
        }
        print("PASS native non-fixture binding and secure-input refusal")
        do {
            let emoji = try textUnits("👩‍💻")
            precondition(emoji == [55357, 56425, 8205, 55357, 56507])
        } catch { print("FAIL joined emoji refused"); exit(1) }
        print("PASS joined emoji preserved")
        let epoch = "00000000000000e1"
        let state = pairedReply(["id": 1, "epoch": epoch, "method": "host.computer", "params": ["op": "state"]], epoch: epoch)
        let paired = state.reply["result"] as? [String: Any] ?? [:]
        let generation = paired["generation"] as? String, capture = paired["capture"] as? String
        precondition(state.reply["ok"] as? Bool == true && !state.stop)
        precondition(generation == epoch && capture == "unavailable")
        for request: [String: Any] in [["id": 2, "epoch": "stale", "method": "heartbeat"],
                                       ["id": 3, "epoch": epoch, "method": "host.computer", "params": ["op": "input"]]] {
            let refused = pairedReply(request, epoch: epoch)
            precondition(refused.reply["ok"] as? Bool == false && !refused.stop)
        }
        precondition(pairedReply(["id": 4, "epoch": epoch, "method": "stop"], epoch: epoch).stop)
        print("PASS paired identity mode: kernel epoch, stale/unsupported refusal, stop")
        for info: [String: Any] in [[:], ["CharioxDrillKernelCDHash": "abc"]] {
            do { _ = try kernelRequirement(info); print("FAIL unpinned kernel admitted"); exit(1) } catch Refusal.disabled { }
        }
        let pinned = try kernelRequirement(["CharioxDrillKernelCDHash": String(repeating: "A", count: 40)])
        precondition(pinned == "cdhash H\"" + String(repeating: "a", count: 40) + "\"")
        print("PASS pairing requires an allowlisted kernel identity")
    }
}
