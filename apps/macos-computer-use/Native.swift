import AppKit
import ApplicationServices
import Carbon
import ScreenCaptureKit
import Security

// AX/WindowServer reads are injected here; admission and release share the
// same destination checks. Cleanup bypasses Action authority, never destination.
struct NativeDestinationFence<Element> {
    let validateTarget: (Request) throws -> Void
    let pointerPoint: () throws -> CGPoint
    let selectedWindow: (Request) throws -> Element
    let focusedElement: (Element) throws -> Element
    let bind: (Element, Element) throws -> Void
    let geometry: (Element) throws -> ClickGeometry
    let windowBounds: (Request) throws -> CGRect
    let bounds: (Element) throws -> CGRect
    let role: (Element) throws -> String?
    let hitAt: (CGPoint) throws -> Element
    let parent: (Element) throws -> Element?
    let equal: (Element, Element) -> Bool
    let occlusion: (Request, CGPoint, CGRect) throws -> Void
    let authority: () throws -> Void
    func check(_ request: Request, element: Element, typing: Bool,
               clickGeometry: ClickGeometry? = nil, eventLocation: CGPoint? = nil,
               physicalKey: Bool = false, checkAuthority: Bool = true) throws {
        try validateTarget(request)
        if request.clickAtPointer {
            try checkPointerSnapshot(request, currentPoint: try pointerPoint())
        }
        let window = try selectedWindow(request)
        let focused = try focusedElement(window)
        try bind(element, window)
        let geometry = try self.geometry(element)
        guard try bounds(window) == geometry.windowBounds else { throw tagged(.target, "window-bounds-changed") }
        _ = try geometry.checkedLocation(geometry.location, current: geometry)
        if request.ownerWindow {
            guard equal(focused, element) else { throw tagged(.target, "owner-focus-changed") }
        }
        if typing {
            if let clickGeometry {
                _ = try clickGeometry.checkedLocation(clickGeometry.location, current: geometry)
            }
            guard equal(focused, element), request.ownerWindow || request.target == "ordinary" || (physicalKey && request.target == "text"),
                  let role = try role(element),
                  [kAXTextFieldRole, kAXTextAreaRole].contains(role) else { throw tagged(.target, "typing-target-focus-or-role") }
        } else {
            let location: CGPoint
            if let clickGeometry {
                guard let eventLocation, try windowBounds(request) == geometry.windowBounds else { throw tagged(.target, "saved-window-changed") }
                location = try clickGeometry.checkedLocation(eventLocation, current: geometry)
            } else { location = geometry.location }
            let hit = try hitAt(location)
            var candidate = hit, matched = false
            for _ in 0..<8 {
                if equal(candidate, element) { matched = true; break }
                guard request.target == "scroll", let parent = try parent(candidate) else { break }
                candidate = parent
            }
            guard matched else { throw tagged(.target, "hit-target-mismatch") }
            if let clickGeometry {
                // AX hit-testing can itself race a move or resize. Recheck its snapshot.
                _ = try clickGeometry.checkedLocation(location, current:
                    self.geometry(element))
                guard try bounds(window) == clickGeometry.windowBounds else { throw tagged(.target, "window-changed-after-hit") }
            }
        }
        // AX hit-testing is app-scoped. A foreign nonactivating panel can leave
        // both focus and AX hits unchanged; prove visibility using current CG order.
        try occlusion(request, geometry.location, geometry.windowBounds)
        if checkAuthority { try authority() }
    }
}
struct MacSource: NativeSource {
    var authority: () throws -> Void = {}
    var tagInput: (CGEvent) -> Void = { _ in }

    func checkPermission(_ operation: Operation) throws {
        try authority()
        try checkOperationPermissions(operation, screenCaptureAccess: {
            if #available(macOS 14.0, *) { return CGPreflightScreenCaptureAccess() }
            return false
        }, accessibilityAccess: {
            AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: false] as CFDictionary)
        })
    }
    func validateTarget(_ request: Request) throws {
        guard let app = NSRunningApplication(processIdentifier: request.pid), !app.isTerminated else { throw tagged(.target, "process-unavailable") }
        if request.ownerWindow {
            try checkOwnerTarget(bundleIdentifier: app.bundleIdentifier, pid: request.pid)
        } else {
            let expected = Bundle.main.bundleURL.deletingLastPathComponent()
                .appendingPathComponent("Chariox Computer Fixture.app/Contents/MacOS/fixture").resolvingSymlinksInPath()
            guard app.bundleIdentifier == "ai.chariox.computer-fixture",
                  app.executableURL?.resolvingSymlinksInPath() == expected else { throw tagged(.target, "fixture-code-path") }
            var code: SecStaticCode?
            guard SecStaticCodeCreateWithPath(expected as CFURL, [], &code) == errSecSuccess,
                  let code, SecStaticCodeCheckValidity(code, [], nil) == errSecSuccess else { throw tagged(.target, "fixture-signature") }
        }
        guard !IsSecureEventInputEnabled() else { throw tagged(.secure, "secure-event-input") }
        _ = try windowBounds(request)
    }
    func windowBounds(_ request: Request) throws -> CGRect {
        guard let info = CGWindowListCopyWindowInfo([.optionIncludingWindow], request.window) as? [[String: Any]],
              info.count == 1, info[0][kCGWindowOwnerPID as String] as? Int == Int(request.pid),
              info[0][kCGWindowIsOnscreen as String] as? Bool == true,
              let bounds = info[0][kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { throw tagged(.target, "cg-window-unavailable") }
        return rect
    }
    func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
        var value: CFTypeRef?
        guard AXUIElementSetMessagingTimeout(element, 0.5) == .success else {
            throw tagged(.target, "ax-attribute-timeout")
        }
        let status = AXUIElementCopyAttributeValue(element, name as CFString, &value)
        guard status == .success, let value else {
            throw tagged(.target, "ax-attribute-\(name)-status-\(status.rawValue)")
        }
        return value
    }
    func target(_ request: Request) throws -> AXUIElement {
        try validateTarget(request)
        let app = AXUIElementCreateApplication(request.pid)
        guard AXUIElementSetMessagingTimeout(app, 0.5) == .success else { throw tagged(.target, "ax-app-timeout") }
        let window = try selectedWindow(request, app: app)
        if request.ownerWindow { return try focusedElement(app: app, window: window) }
        guard try attribute(window, kAXTitleAttribute) as? String == "Chariox public fixture" else { throw tagged(.target, "fixture-window-title") }
        var pending = [window], visited = 0
        while let element = pending.popLast(), visited < 128 {
            visited += 1
            guard AXUIElementSetMessagingTimeout(element, 0.5) == .success else { throw tagged(.target, "ax-element-timeout") }
            if (try? attribute(element, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole { continue }
            if (try? attribute(element, kAXIdentifierAttribute) as? String) == request.target { return element }
            if let children = try? attribute(element, kAXChildrenAttribute) as? [AXUIElement] { pending += children.prefix(128 - visited) }
        }
        throw tagged(.target, "fixture-target-not-found")
    }
    func bounds(_ element: AXUIElement) throws -> CGRect {
        let position = try attribute(element, kAXPositionAttribute), size = try attribute(element, kAXSizeAttribute)
        guard CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw tagged(.target, "bounds-not-axvalue") }
        var origin = CGPoint.zero, extent = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &origin),
              AXValueGetValue(size as! AXValue, .cgSize, &extent), extent.width > 0, extent.height > 0 else { throw tagged(.target, "invalid-ax-bounds") }
        return CGRect(origin: origin, size: extent)
    }
    func geometry(_ request: Request, element: AXUIElement) throws -> ClickGeometry {
        let window = try windowBounds(request), elementBounds = try bounds(element)
        var displays = [CGDirectDisplayID](repeating: 0, count: 32), count: UInt32 = 0
        guard CGGetActiveDisplayList(32, &displays, &count) == .success, count > 0, count <= 32 else { throw tagged(.target, "active-display-list") }
        let visible = try visibleWindowFrame(window: window, element: elementBounds,
            displays: displays.prefix(Int(count)).map { CGDisplayBounds($0) }, point: request.clickPoint)
        return ClickGeometry(windowBounds: window, elementBounds: elementBounds,
                             visibleWindowBounds: visible, requestedPoint: request.clickPoint)
    }
    func selectedWindow(_ request: Request, app: AXUIElement) throws -> AXUIElement {
        // Refuse focus loss before asking an inactive app for focused AX state.
        // The final binding check repeats this after window enumeration.
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == request.pid else {
            throw tagged(.target, "not-frontmost")
        }
        let rect = try windowBounds(request)
        guard let windows = try attribute(app, kAXWindowsAttribute) as? [AXUIElement], windows.count <= 32 else { throw tagged(.target, "ax-window-list") }
        // Public AX exposes geometry, not a CG window ID. Refuse ambiguous matches.
        let matches = try windows.filter { try bounds($0) == rect }
        // Another CG window at the same bounds would make the AX binding ambiguous.
        guard let all = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else { throw tagged(.target, "cg-window-list") }
        let same = all.filter {
            guard $0[kCGWindowOwnerPID as String] as? Int == Int(request.pid),
                  let value = $0[kCGWindowBounds as String] as? [String: Any] else { return false }
            return CGRect(dictionaryRepresentation: value as CFDictionary) == rect
        }
        let focusedWindow = try attribute(app, kAXFocusedWindowAttribute)
        try checkWindowBinding(pid: request.pid,
                               frontmostPID: NSWorkspace.shared.frontmostApplication?.processIdentifier,
                               axMatches: matches.count, cgMatches: same.count,
                               focused: matches.first.map { CFEqual(focusedWindow, $0) } ?? false)
        return matches[0]
    }
    func focusedElement(app: AXUIElement, window: AXUIElement) throws -> AXUIElement {
        let focused = try attribute(app, kAXFocusedUIElementAttribute)
        guard CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw tagged(.target, "focused-element-type") }
        let element = focused as! AXUIElement
        var candidate = element
        // Inspect roles only. Never read a focused field's value or label.
        for _ in 0..<32 {
            if (try? attribute(candidate, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole { throw tagged(.secure, "secure-focused-ancestor") }
            if CFEqual(candidate, window) { return element }
            let parent = try attribute(candidate, kAXParentAttribute)
            guard CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw tagged(.target, "focused-parent-type") }
            candidate = parent as! AXUIElement
        }
        throw tagged(.target, "focused-element-outside-window")
    }
    func destinationFence(for request: Request) -> NativeDestinationFence<AXUIElement> {
        let app = AXUIElementCreateApplication(request.pid)
        return NativeDestinationFence(validateTarget: validateTarget, pointerPoint: currentPointerPoint,
            selectedWindow: { try selectedWindow($0, app: app) },
            focusedElement: { try focusedElement(app: app, window: $0) },
            bind: { try bind($0, to: $1, pid: request.pid) },
            geometry: { try geometry(request, element: $0) }, windowBounds: windowBounds, bounds: bounds,
            role: { try attribute($0, kAXRoleAttribute) as? String },
            hitAt: { point in
                var hit: AXUIElement?
                guard AXUIElementCopyElementAtPosition(app, Float(point.x), Float(point.y), &hit) == .success,
                      let hit else { throw tagged(.target, "no-hit-at-point") }
                return hit
            }, parent: { node in
                guard let parent = try? attribute(node, kAXParentAttribute),
                      CFGetTypeID(parent) == AXUIElementGetTypeID() else { return nil }
                return (parent as! AXUIElement)
            }, equal: { CFEqual($0, $1) },
            occlusion: { try checkWindowOcclusion($0, point: $1, windowBounds: $2) }, authority: authority)
    }
    func fence(_ request: Request, element: AXUIElement, typing: Bool,
               clickGeometry: ClickGeometry? = nil, eventLocation: CGPoint? = nil, physicalKey: Bool = false) throws {
        try destinationFence(for: request).check(request, element: element, typing: typing,
            clickGeometry: clickGeometry, eventLocation: eventLocation, physicalKey: physicalKey)
    }
    func bind(_ element: AXUIElement, to ancestor: AXUIElement, pid: pid_t) throws {
        var actual: pid_t = 0, candidate = element
        guard AXUIElementGetPid(element, &actual) == .success, actual == pid else { throw tagged(.target, "element-pid-mismatch") }
        for _ in 0..<32 {
            if (try? attribute(candidate, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole { throw tagged(.secure, "secure-bound-ancestor") }
            if CFEqual(candidate, ancestor) { return }
            let parent = try attribute(candidate, kAXParentAttribute)
            guard CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw tagged(.target, "bound-parent-type") }
            candidate = parent as! AXUIElement
        }
        throw tagged(.target, "element-outside-ancestor")
    }
    func fixtureCounter(_ request: Request, element: AXUIElement) -> Int? {
        guard !request.ownerWindow, request.target == "button",
              let title = try? attribute(element, kAXTitleAttribute) as? String,
              title.hasPrefix("Clicks: "), title.count <= 24 else { return nil }
        return Int(title.dropFirst(8))
    }
    func scrollValue(_ scroller: AXUIElement) throws -> Double {
        guard let number = try attribute(scroller, kAXValueAttribute) as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID(), number.doubleValue.isFinite,
              (0...1).contains(number.doubleValue) else { throw tagged(.target, "invalid-scroll-value") }
        return number.doubleValue
    }
    func performAX(_ path: InputPath, operation: Operation, request: Request,
                   element: AXUIElement) async throws -> String {
        var scroller: AXUIElement?, before: Double?, counter: Int?
        if path == .axPress {
            var names: CFArray?
            guard AXUIElementCopyActionNames(element, &names) == .success,
                  let actions = names as? [String], actions.contains(kAXPressAction) else { throw tagged(.target, "press-unavailable") }
            counter = fixtureCounter(request, element: element)
        } else {
            let value = try attribute(element, kAXVerticalScrollBarAttribute)
            guard CFGetTypeID(value) == AXUIElementGetTypeID() else { throw tagged(.target, "scrollbar-type") }
            scroller = (value as! AXUIElement)
            guard let scroller, try attribute(scroller, kAXRoleAttribute) as? String == kAXScrollBarRole else { throw tagged(.target, "scrollbar-role") }
            try bind(scroller, to: element, pid: request.pid)
            var settable: DarwinBoolean = false
            guard AXUIElementIsAttributeSettable(scroller, kAXValueAttribute as CFString, &settable) == .success,
                  settable.boolValue else { throw tagged(.target, "scrollbar-not-settable") }
            before = try scrollValue(scroller)
        }
        try checkPermission(operation)
        try fence(request, element: element, typing: false)
        if let scroller, let before {
            try bind(scroller, to: element, pid: request.pid)
            guard try scrollValue(scroller) == before else { throw tagged(.target, "scroll-value-changed") }
            try fence(request, element: element, typing: false)
            guard AXUIElementSetAttributeValue(scroller, kAXValueAttribute as CFString,
                NSNumber(value: try nextScrollValue(before))) == .success else { throw tagged(.native, "scroll-mutation-failed") }
        } else {
            guard AXUIElementPerformAction(element, kAXPressAction as CFString) == .success else { throw tagged(.native, "press-mutation-failed") }
        }
        // Poll only the synthetic counter or numeric scrollbar. Never field text.
        for _ in 0..<4 {
            do { try await Task.sleep(nanoseconds: 50_000_000) }
            catch { throw tagged(.native, "ax-observation-cancelled") }
            try checkPermission(operation)
            try fence(request, element: element, typing: false)
            if let scroller, let before {
                try bind(scroller, to: element, pid: request.pid)
                if try scrollValue(scroller) > before {
                    return inputReceipt(path: "AXScrollValue", observed: "observed vertical scroll position increased")
                }
            } else if let counter, fixtureCounter(request, element: element) == counter + 1 {
                return inputReceipt(path: "AXPress", observed: "observed fixture counter increment")
            }
        }
        return inputReceipt(path: path == .axPress ? "AXPress" : "AXScrollValue")
    }
    func perform(_ operation: Operation, request: Request) async throws -> String {
        let element = try target(request)
        try fence(request, element: element, typing: false)
        if case .capture(let path) = operation {
            guard #available(macOS 14.0, *) else { throw tagged(.permission, "capture-os-unavailable") }
            let content: SCShareableContent
            do { content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true) }
            catch { throw tagged(.native, "shareable-content-api") }
            guard let window = content.windows.first(where: { $0.windowID == request.window && $0.owningApplication?.processID == request.pid })
            else { throw tagged(.target, "capture-window-unavailable") }
            let filter = SCContentFilter(desktopIndependentWindow: window), config = SCStreamConfiguration()
            config.width = Int(filter.contentRect.width * CGFloat(filter.pointPixelScale))
            config.height = Int(filter.contentRect.height * CGFloat(filter.pointPixelScale))
            config.showsCursor = false; config.capturesAudio = false
            if #available(macOS 15.0, *) { config.captureMicrophone = false }
            let image: CGImage
            do { image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config) }
            catch { throw tagged(.native, "screenshot-api") }
            try fence(request, element: element, typing: false)
            guard CGPreflightScreenCaptureAccess() else { throw tagged(.permission, "capture-permission-revoked") }
            let bitmap = NSBitmapImageRep(cgImage: image)
            guard let png = bitmap.representation(using: .png, properties: [:]) else { throw tagged(.native, "png-encoding-failed") }
            do { try png.write(to: URL(fileURLWithPath: path), options: .atomic) }
            catch { throw tagged(.native, "frame-write-failed") }
            return "frame \(image.width)x\(image.height) window=\(request.window)"
        }
        if operation == .read {
            if request.publicTextDrillDocument != nil {
                return try publicTextDrillReference(request, element: element)
            }
            // Read only public metadata, never values, labels, selections or secure descendants.
            guard let role = try attribute(element, kAXRoleAttribute) as? String,
                  [kAXTextFieldRole, kAXTextAreaRole, kAXButtonRole, kAXScrollAreaRole].contains(role) else { throw tagged(.target, "unsupported-read-role") }
            return "target=\(request.target) role=\(role)"
        }
        guard let role = try attribute(element, kAXRoleAttribute) as? String else { throw tagged(.target, "missing-input-role") }
        let path = try inputPath(operation, role: role)
        if path == .axPress || path == .axScrollValue {
            return try await performAX(path, operation: operation, request: request, element: element)
        }
        if operation == .click {
            return try await performTextClick(request, element: element,
                                              geometry: geometry(request, element: element))
        }
        guard case .text(let text) = operation else { throw tagged(.arguments, "expected-text-operation") }
        let source = CGEventSource(stateID: .privateState)
        let units = try textUnits(text)
        var events: [CGEvent] = []
        for down in [true, false] {
            // Keycode 0 is experimental; inertness is an owner-session gate.
            guard let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down) else { throw tagged(.native, "keyboard-event-create") }
            units.withUnsafeBufferPointer { event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: $0.baseAddress) }
            events.append(event)
        }
        guard CGPreflightPostEventAccess() else { throw tagged(.permission, "post-permission-denied") }
        try dispatchInputEvents(events, fence: { _ in
            try fence(request, element: element, typing: true)
        }, releaseFence: { _ in
            try fence(request, element: element, typing: true)
            try checkPermission(operation)
        }, releaseAllowed: { CGPreflightPostEventAccess() }, post: { tagInput($0); $0.postToPid(request.pid) })
        try fence(request, element: element, typing: true)
        return inputReceipt(path: "CGEventPIDText")
    }
}

// TextEdit-only discovery, no capture, AX reads, window titles or automatic selection.
func ownerWindowChoices() throws -> [[String: Any]] {
    guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { throw tagged(.native, "owner-window-list") }
    return windows.compactMap { info in
        guard let pid = info[kCGWindowOwnerPID as String] as? Int,
              let id = info[kCGWindowNumber as String] as? UInt32,
              info[kCGWindowLayer as String] as? Int == 0,
              let app = NSRunningApplication(processIdentifier: Int32(pid)),
              !app.isTerminated, let bundle = app.bundleIdentifier,
              (try? checkOwnerTarget(bundleIdentifier: bundle, pid: Int32(pid))) != nil
        else { return nil }
        return ["pid": pid, "windowID": id, "bundle": bundle]
    }
}
