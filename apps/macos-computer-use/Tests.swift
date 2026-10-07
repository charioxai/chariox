import Foundation

final class FakeSource: NativeSource {
    var calls = 0
    var typed: [UInt16] = []
    var secureInput = false
    func validateFixture(_ request: Request) throws {
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
        let source = FakeSource()
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
        for text in ["", "\n", "\t", "\u{1b}", "\u{2028}", String(repeating: "a", count: 65)] {
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
