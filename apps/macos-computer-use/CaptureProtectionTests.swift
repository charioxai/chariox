import AppKit

@main struct CaptureProtectionTests {
    static func main() throws {
        var publications = 0
        for result: (AXError, CFTypeRef?) in [(.cannotComplete, nil), (.failure, nil), (.success, NSNumber(value: 7)), (.success, nil)] {
            var refused = false
            do {
                _ = try captureElementIsSecure { result }
                publications += 1
            } catch { refused = true }
            precondition(refused && publications == 0, "failed classification published an image")
        }
        let secure = try captureElementIsSecure { (.success, kAXSecureTextFieldSubrole as CFString) }
        let absent = try captureElementIsSecure { (.attributeUnsupported, nil) }
        let noValue = try captureElementIsSecure { (.noValue, nil) }
        precondition(secure && !absent && !noValue)
        print("PASS secure AX classification fails closed for messaging/type/missing-value errors before publication")
    }
}
