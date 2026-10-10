import AppKit
import Security

func launchIdentity() throws -> [String: Any] {
    var code: SecCode?, staticCode: SecStaticCode?, info: CFDictionary?
    guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
          SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
          SecCodeCopySigningInformation(staticCode, signingInformationFlags(), &info) == errSecSuccess,
          let data = info as? [String: Any] else { throw tagged(.native, "launch-signature-info") }
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
            if arguments.count == 2, arguments[0] == "--pair" { try await runPairing(arguments[1]) }
            print(String(data: try JSONSerialization.data(withJSONObject: launchIdentity(), options: [.sortedKeys]), encoding: .utf8)!)
            if arguments == ["--identity"] { return }
            if arguments == ["--drill-permissions"] {
                try MacSource().checkPermission(.read)
                print("drillPermission accessibility=granted")
                return
            }
            if arguments == ["--enable-owner-window", "--list-windows"] {
                print(String(data: try JSONSerialization.data(withJSONObject: ownerWindowChoices(), options: [.sortedKeys]), encoding: .utf8)!)
                return
            }
            var request = try Request.parse(arguments)
            if request.publicTextDrillDocument != nil {
                // Compare the historical sampling order before target discovery.
                request.publicDrillLegacyPointer = CGEvent(source: nil)?.location
                request.publicDrillAppKitPointer = try currentPointerPoint()
            }
            if request.clickAtPointer {
                // Read the existing pointer once. This does not post a mouse event.
                request.clickPoint = try currentPointerPoint()
            }
            for receipt in try await run(request, source: MacSource()) { print(receipt) }
        } catch {
            // Never print framework errors or app-supplied AX data.
            fputs("\(refusalMessage(error))\n", stderr)
            exit(1)
        }
    }
}
