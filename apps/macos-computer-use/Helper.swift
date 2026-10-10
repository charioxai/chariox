import AppKit
import Security

func launchIdentity() throws -> [String: Any] {
    var code: SecCode?, staticCode: SecStaticCode?, info: CFDictionary?
    guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
          SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
          SecCodeCopySigningInformation(staticCode, signingInformationFlags(), &info) == errSecSuccess,
          let data = info as? [String: Any] else { throw Refusal.native }
    return ["path": Bundle.main.executableURL!.resolvingSymlinksInPath().path,
            "identifier": data[kSecCodeInfoIdentifier as String] ?? "unsigned",
            "team": data[kSecCodeInfoTeamIdentifier as String] ?? "none (ad-hoc)",
            "cdhash": (data[kSecCodeInfoUnique as String] as? Data)?.map { String(format: "%02x", $0) }.joined() ?? "none",
            "signatureValid": SecCodeCheckValidity(code, [], nil) == errSecSuccess,
            "pid": getpid(), "parentPID": getppid()]
}
@main struct Helper {
    @MainActor static func main() async {
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            print(String(data: try JSONSerialization.data(withJSONObject: launchIdentity(), options: [.sortedKeys]), encoding: .utf8)!)
            if arguments == ["--identity"] { return }
            if arguments == ["--enable-owner-window", "--list-windows"] {
                print(String(data: try JSONSerialization.data(withJSONObject: ownerWindowChoices(), options: [.sortedKeys]), encoding: .utf8)!)
                return
            }
            var request = try Request.parse(arguments)
            if request.clickAtPointer {
                // Read the existing pointer once. This does not post a mouse event.
                guard let point = CGEvent(source: nil)?.location, point.x.isFinite, point.y.isFinite else { throw Refusal.native }
                request.clickPoint = point
            }
            for receipt in try await run(request, source: MacSource()) { print(receipt) }
        } catch {
            // Never print framework errors or app-supplied AX data.
            fputs("\(refusalMessage(error))\n", stderr)
            exit(1)
        }
    }
}
