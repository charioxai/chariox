import AppKit
import ApplicationServices

// A diagnostic for one exact public document, never a general AX value dump.
let publicProbeText = (1...12).map { String(format: "line %02d public caret probe", $0) }.joined(separator: "\n") + "\n"

extension MacSource {
    func publicTextDrillReference(_ request: Request, element: AXUIElement) throws -> String {
        guard let path = request.publicTextDrillDocument, path.hasPrefix("/"),
              request.ownerWindow, request.operations == [.read] else { throw tagged(.arguments, "public-drill-scope") }
        let expected = URL(fileURLWithPath: path).standardizedFileURL.resolvingSymlinksInPath()
        let root = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".codex/evidence/browser-computer-use-macos", isDirectory: true)
            .resolvingSymlinksInPath().path + "/"
        guard expected.path.hasPrefix(root), expected.lastPathComponent == "caret-probe-r4.txt",
              (try? String(contentsOf: expected, encoding: .utf8)) == publicProbeText else {
            throw tagged(.target, "not-public-drill-file")
        }
        let app = AXUIElementCreateApplication(request.pid)
        let window = try selectedWindow(request, app: app)
        guard let documentString = try attribute(window, kAXDocumentAttribute) as? String,
              let document = URL(string: documentString)?.standardizedFileURL.resolvingSymlinksInPath(),
              document == expected else { throw tagged(.target, "public-drill-document-mismatch") }
        guard let count = try attribute(element, kAXNumberOfCharactersAttribute) as? NSNumber,
              count.intValue == publicProbeText.utf16.count else { throw tagged(.target, "public-drill-length-mismatch") }
        guard let text = try attribute(element, kAXValueAttribute) as? String,
              text == publicProbeText else { throw tagged(.target, "public-drill-content-mismatch") }
        try fence(request, element: element, typing: false)
        if request.closePublicTextDrill {
            guard let edited = try attribute(window, kAXEditedAttribute) as? Bool, !edited else {
                throw tagged(.target, "public-drill-document-edited")
            }
            let closeValue = try attribute(window, kAXCloseButtonAttribute)
            guard CFGetTypeID(closeValue) == AXUIElementGetTypeID() else { throw tagged(.target, "public-drill-close-type") }
            let close = closeValue as! AXUIElement
            try bind(close, to: window, pid: request.pid)
            try checkPermission(.click)
            try fence(request, element: element, typing: false)
            guard AXUIElementPerformAction(close, kAXPressAction as CFString) == .success else {
                throw tagged(.native, "public-drill-close-failed")
            }
            return "closed public TextEdit drill window=\(request.window)"
        }
        func parameter(_ name: String, range: CFRange) throws -> CFTypeRef {
            var range = range, result: CFTypeRef?
            guard let value = AXValueCreate(.cfRange, &range),
                  AXUIElementCopyParameterizedAttributeValue(element, name as CFString, value, &result) == .success,
                  let result else { throw tagged(.target, "public-drill-range-metadata") }
            return result
        }
        func rect(_ index: Int) throws -> CGRect {
            let value = try parameter(kAXBoundsForRangeParameterizedAttribute, range: CFRange(location: index, length: 1))
            var rect = CGRect.zero
            guard CFGetTypeID(value) == AXValueGetTypeID(),
                  AXValueGetValue(value as! AXValue, .cgRect, &rect),
                  !rect.isEmpty, !rect.isNull else { throw tagged(.target, "public-drill-range-bounds") }
            return rect
        }
        // Read font metadata only after matching the entire synthetic value.
        guard let attributed = try parameter(kAXAttributedStringForRangeParameterizedAttribute,
            range: CFRange(location: 0, length: 1)) as? NSAttributedString,
              attributed.length == 1,
              let attributes = attributed.attribute(.accessibilityFont, at: 0, effectiveRange: nil) as? [NSAccessibility.FontAttributeKey: Any],
              let name = attributes[.fontName] as? String,
              let size = attributes[.fontSize] as? NSNumber,
              size.doubleValue.isFinite, size.doubleValue > 0, size.doubleValue <= 128,
              let font = NSFont(name: name, size: size.doubleValue) else {
            throw tagged(.target, "public-drill-font-metadata")
        }
        let replica = NSTextView(frame: NSRect(x: 0, y: 0, width: 1000, height: 1000))
        replica.font = font; replica.string = publicProbeText
        guard let layout = replica.layoutManager, let container = replica.textContainer else {
            throw tagged(.native, "public-drill-appkit-layout")
        }
        layout.ensureLayout(for: container)
        let firstAX = try rect(0)
        let firstLocal = layout.boundingRect(forGlyphRange: NSRange(location: 0, length: 1), in: container)
        var probes: [[String: Any]] = []
        for (name, index) in [("line5-before-r", 125), ("line5-after-r", 126)] {
            let bounds = try rect(index)
            let point = CGPoint(x: bounds.minX + bounds.width * 0.25, y: bounds.midY)
            let local = CGPoint(x: point.x - firstAX.minX + firstLocal.minX + replica.textContainerOrigin.x,
                                y: point.y - firstAX.minY + firstLocal.minY + replica.textContainerOrigin.y)
            let reference = replica.characterIndexForInsertion(at: local)
            guard reference == index, try rangeAtPoint(element, point: point)?.location == index else {
                throw tagged(.target, "public-drill-independent-reference-mismatch")
            }
            probes.append(["name": name, "expectedLocation": index, "referenceLocation": reference,
                           "point": ["x": point.x, "y": point.y],
                           "axBounds": ["x": bounds.minX, "y": bounds.minY, "width": bounds.width, "height": bounds.height]])
        }
        let selected = try rangeValue(attribute(element, kAXSelectedTextRangeAttribute))
        let bounds = try windowBounds(request)
        let primaryTop = NSScreen.screens[0].frame.maxY
        let cocoaWindow = CGRect(x: bounds.minX, y: primaryTop - bounds.maxY, width: bounds.width, height: bounds.height)
        let screenIndex = NSScreen.screens.indices.max(by: {
            let a = NSScreen.screens[$0].frame.intersection(cocoaWindow)
            let b = NSScreen.screens[$1].frame.intersection(cocoaWindow)
            return a.width * a.height < b.width * b.height
        })!
        guard let legacy = CGEvent(source: nil)?.location else { throw tagged(.native, "public-drill-pointer-event") }
        let pointer = try currentPointerPoint()
        guard let launchLegacy = request.publicDrillLegacyPointer, let launchAppKit = request.publicDrillAppKitPointer else {
            throw tagged(.native, "public-drill-launch-pointer")
        }
        let result: [String: Any] = ["probes": probes,
            "selectedRange": ["location": selected.location, "length": selected.length],
            "windowScreenIndex": screenIndex, "primaryTop": primaryTop,
            "fontName": font.fontName, "fontSize": font.pointSize,
            "launchLegacyCGPointer": ["x": launchLegacy.x, "y": launchLegacy.y],
            "launchAppKitPointer": ["x": launchAppKit.x, "y": launchAppKit.y],
            "legacyCGPointer": ["x": legacy.x, "y": legacy.y],
            "appKitPointer": ["x": pointer.x, "y": pointer.y]]
        try fence(request, element: element, typing: false)
        return "publicTextDrillReference " + String(data: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]), encoding: .utf8)!
    }
}
