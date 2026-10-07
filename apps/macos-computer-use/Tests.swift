import Foundation
import Security

final class FakeSource: NativeSource {
    var calls = 0
    var typed: [UInt16] = []
    var secureInput = false
    func validateTarget(_ request: Request) throws {
        guard request.pid == 42, request.window == 1 else { throw Refusal.target }
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
        guard signingInformationFlags().rawValue & kSecCSSigningInformation != 0 else {
            print("FAIL signing information flag missing"); exit(1)
        }
        do {
            _ = try textUnits(String(repeating: "😀", count: 11))
            print("FAIL 22-unit emoji payload admitted"); exit(1)
        } catch Refusal.text { }
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
        try checkWindowBinding(pid: 42, ownerPID: 42, frontmostPID: 42, axMatches: 1, cgMatches: 1, focused: true)
        for facts in [(43 as Int32, 42 as Int32?, 1, 1, true), (42, 43, 1, 1, true),
                      (42, nil, 1, 1, true), (42, 42, 0, 1, true), (42, 42, 2, 1, true),
                      (42, 42, 1, 2, true), (42, 42, 1, 1, false)] {
            do {
                try checkWindowBinding(pid: 42, ownerPID: facts.0, frontmostPID: facts.1,
                                       axMatches: facts.2, cgMatches: facts.3, focused: facts.4)
                print("FAIL unsafe window binding admitted"); exit(1)
            } catch Refusal.target { }
        }
        print("PASS window ownership, frontmost, unique geometry and AX window focus fences")
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
    }
}
