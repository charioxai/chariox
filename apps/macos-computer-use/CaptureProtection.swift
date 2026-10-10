import ApplicationServices

// The capture protection boundary distinguishes absent AX metadata from a failed
// classification. Messaging failures and invalid types never become public pixels.
func captureElementIsSecure(read: () -> (AXError, CFTypeRef?)) throws -> Bool {
    let (status, value) = read()
    if status == .attributeUnsupported || status == .noValue { return false }
    guard status == .success, let value, CFGetTypeID(value) == CFStringGetTypeID(),
          let subrole = value as? String else { throw tagged(.secure, "capture-ax-classification-unknown") }
    return subrole == kAXSecureTextFieldSubrole
}
