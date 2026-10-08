import Security

func signingInformationFlags() -> SecCSFlags { SecCSFlags(rawValue: kSecCSSigningInformation) }

// Shared admission gate for owner operations and discovery.
func checkOwnerTarget(bundleIdentifier: String?, pid: Int32,
                      satisfiesRequirement: (Int32, String) -> Bool = runningCodeSatisfiesRequirement) throws {
    guard pid > 0, bundleIdentifier == "com.apple.TextEdit",
          satisfiesRequirement(pid, "identifier \"com.apple.TextEdit\" and anchor apple")
    else { throw Refusal.target }
}

func runningCodeSatisfiesRequirement(pid: Int32, requirement: String) -> Bool {
    var code: SecCode?, rule: SecRequirement?
    let attributes = [kSecGuestAttributePid as String: pid] as CFDictionary
    guard SecCodeCopyGuestWithAttributes(nil, attributes, [], &code) == errSecSuccess,
          let code, SecRequirementCreateWithString(requirement as CFString, [], &rule) == errSecSuccess,
          let rule else { return false }
    return SecCodeCheckValidity(code, [], rule) == errSecSuccess
}
