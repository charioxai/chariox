import AppKit
import ApplicationServices

// AX caret placement reads only range metadata, never document text.
extension MacSource {
    func rangeValue(_ value: CFTypeRef) throws -> CFRange {
        guard CFGetTypeID(value) == AXValueGetTypeID() else { throw Refusal.target }
        var range = CFRange()
        guard AXValueGetType(value as! AXValue) == .cfRange,
              AXValueGetValue(value as! AXValue, .cfRange, &range) else { throw Refusal.target }
        return range
    }
    func rangeAtPoint(_ element: AXUIElement, point: CGPoint) throws -> CFRange? {
        var point = point, value: CFTypeRef?
        guard let parameter = AXValueCreate(.cgPoint, &point) else { throw Refusal.native }
        let status = AXUIElementCopyParameterizedAttributeValue(element,
            kAXRangeForPositionParameterizedAttribute as CFString, parameter, &value)
        switch status {
        case .parameterizedAttributeUnsupported, .attributeUnsupported, .noValue, .notImplemented:
            return nil
        case .success:
            guard let value else { throw Refusal.target }
            return try rangeValue(value)
        default: throw Refusal.native
        }
    }
    func performTextClick(_ request: Request, element: AXUIElement,
                          geometry: ClickGeometry) async throws -> String {
        func checked() throws {
            try checkPermission(.click)
            try fence(request, element: element, typing: false,
                      clickGeometry: geometry, eventLocation: geometry.location)
        }
        func resolved() throws -> TextClickResolution {
            try resolveTextClick(at: geometry.location) { try rangeAtPoint(element, point: $0) }
        }
        try checked()
        let resolution = try resolved()
        try checked()
        guard case .selection(let index) = resolution else { throw Refusal.target }
        var settable: DarwinBoolean = false
        guard AXUIElementIsAttributeSettable(element, kAXSelectedTextRangeAttribute as CFString, &settable) == .success,
              settable.boolValue else { throw Refusal.target }
        let app = AXUIElementCreateApplication(request.pid)
        let alreadyFocused = CFEqual(try attribute(app, kAXFocusedUIElementAttribute), element)
        var focusSettable: DarwinBoolean = false
        let focusStatus = AXUIElementIsAttributeSettable(element, kAXFocusedAttribute as CFString, &focusSettable)
        if focusStatus == .success && focusSettable.boolValue {
            try checked()
            guard AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success
            else { throw Refusal.native }
        } else if !alreadyFocused { throw Refusal.target }
        guard CFEqual(try attribute(app, kAXFocusedUIElementAttribute), element) else { throw Refusal.target }
        var caret = CFRange(location: index, length: 0)
        guard let value = AXValueCreate(.cfRange, &caret) else { throw Refusal.native }
        // A failed AX mutation never changes delivery path. Re-resolve after focus
        // so scrolling/layout changes cannot assign a stale character index.
        guard try resolved() == resolution else { throw Refusal.target }
        try checked()
        guard AXUIElementSetAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, value) == .success
        else { throw Refusal.native }
        for _ in 0..<4 {
            try await Task.sleep(nanoseconds: 50_000_000)
            try checked()
            guard CFEqual(try attribute(app, kAXFocusedUIElementAttribute), element) else { throw Refusal.target }
            let selected = try rangeValue(attribute(element, kAXSelectedTextRangeAttribute))
            if selected.location == index && selected.length == 0 {
                return inputReceipt(path: "AXSelectedTextRange", observed:
                    "observed AXSelectedTextRange location=\(index) length=0; point=\(geometry.location.x),\(geometry.location.y)")
            }
        }
        return inputReceipt(path: "AXSelectedTextRange")
    }
}
