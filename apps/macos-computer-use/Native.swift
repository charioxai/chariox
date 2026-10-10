import AppKit
import ApplicationServices
import Carbon
import ScreenCaptureKit
import Security

struct MacSource: NativeSource {
    func checkPermission(_ operation: Operation) throws {
        try checkOperationPermissions(operation, screenCaptureAccess: {
            if #available(macOS 14.0, *) { return CGPreflightScreenCaptureAccess() }
            return false
        }, accessibilityAccess: {
            AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: false] as CFDictionary)
        })
    }
    func validateTarget(_ request: Request) throws {
        guard let app = NSRunningApplication(processIdentifier: request.pid), !app.isTerminated else { throw Refusal.target }
        if request.ownerWindow {
            try checkOwnerTarget(bundleIdentifier: app.bundleIdentifier, pid: request.pid)
        } else {
            let expected = Bundle.main.bundleURL.deletingLastPathComponent()
                .appendingPathComponent("Chariox Computer Fixture.app/Contents/MacOS/fixture").resolvingSymlinksInPath()
            guard app.bundleIdentifier == "ai.chariox.computer-fixture",
                  app.executableURL?.resolvingSymlinksInPath() == expected else { throw Refusal.target }
            var code: SecStaticCode?
            guard SecStaticCodeCreateWithPath(expected as CFURL, [], &code) == errSecSuccess,
                  let code, SecStaticCodeCheckValidity(code, [], nil) == errSecSuccess else { throw Refusal.target }
        }
        guard !IsSecureEventInputEnabled() else { throw Refusal.secure }
        _ = try windowBounds(request)
    }
    func windowBounds(_ request: Request) throws -> CGRect {
        guard let info = CGWindowListCopyWindowInfo([.optionIncludingWindow], request.window) as? [[String: Any]],
              info.count == 1, info[0][kCGWindowOwnerPID as String] as? Int == Int(request.pid),
              info[0][kCGWindowIsOnscreen as String] as? Bool == true,
              let bounds = info[0][kCGWindowBounds as String] as? [String: Any],
              let rect = CGRect(dictionaryRepresentation: bounds as CFDictionary) else { throw Refusal.target }
        return rect
    }
    func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
        var value: CFTypeRef?
        guard AXUIElementSetMessagingTimeout(element, 0.5) == .success,
              AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success,
              let value else { throw Refusal.target }
        return value
    }
    func target(_ request: Request) throws -> AXUIElement {
        try validateTarget(request)
        let app = AXUIElementCreateApplication(request.pid)
        guard AXUIElementSetMessagingTimeout(app, 0.5) == .success else { throw Refusal.target }
        let window = try selectedWindow(request, app: app)
        if request.ownerWindow { return try focusedElement(app: app, window: window) }
        guard try attribute(window, kAXTitleAttribute) as? String == "Chariox public fixture" else { throw Refusal.target }
        var pending = [window], visited = 0
        while let element = pending.popLast(), visited < 128 {
            visited += 1
            guard AXUIElementSetMessagingTimeout(element, 0.5) == .success else { throw Refusal.target }
            if (try? attribute(element, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole { continue }
            if (try? attribute(element, kAXIdentifierAttribute) as? String) == request.target { return element }
            if let children = try? attribute(element, kAXChildrenAttribute) as? [AXUIElement] { pending += children.prefix(128 - visited) }
        }
        throw Refusal.target
    }
    func bounds(_ element: AXUIElement) throws -> CGRect {
        let position = try attribute(element, kAXPositionAttribute), size = try attribute(element, kAXSizeAttribute)
        guard CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw Refusal.target }
        var origin = CGPoint.zero, extent = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &origin),
              AXValueGetValue(size as! AXValue, .cgSize, &extent), extent.width > 0, extent.height > 0 else { throw Refusal.target }
        return CGRect(origin: origin, size: extent)
    }
    func geometry(_ request: Request, element: AXUIElement) throws -> ClickGeometry {
        let window = try windowBounds(request), elementBounds = try bounds(element)
        var displays = [CGDirectDisplayID](repeating: 0, count: 32), count: UInt32 = 0
        guard CGGetActiveDisplayList(32, &displays, &count) == .success, count > 0, count <= 32 else { throw Refusal.target }
        let visible = try visibleWindowFrame(window: window, element: elementBounds,
            displays: displays.prefix(Int(count)).map { CGDisplayBounds($0) }, point: request.clickPoint)
        return ClickGeometry(windowBounds: window, elementBounds: elementBounds,
                             visibleWindowBounds: visible, requestedPoint: request.clickPoint)
    }
    func selectedWindow(_ request: Request, app: AXUIElement) throws -> AXUIElement {
        let rect = try windowBounds(request)
        guard let windows = try attribute(app, kAXWindowsAttribute) as? [AXUIElement], windows.count <= 32 else { throw Refusal.target }
        // Public AX exposes geometry, not a CG window ID. Refuse ambiguous matches.
        let matches = try windows.filter { try bounds($0) == rect }
        // Another CG window at the same bounds would make the AX binding ambiguous.
        guard let all = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else { throw Refusal.target }
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
        guard CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw Refusal.target }
        let element = focused as! AXUIElement
        var candidate = element
        // Inspect roles only. Never read a focused field's value or label.
        for _ in 0..<32 {
            if (try? attribute(candidate, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole { throw Refusal.secure }
            if CFEqual(candidate, window) { return element }
            let parent = try attribute(candidate, kAXParentAttribute)
            guard CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw Refusal.target }
            candidate = parent as! AXUIElement
        }
        throw Refusal.target
    }
    func fence(_ request: Request, element: AXUIElement, typing: Bool,
               clickGeometry: ClickGeometry? = nil, eventLocation: CGPoint? = nil) throws {
        try validateTarget(request)
        let app = AXUIElementCreateApplication(request.pid)
        let window = try selectedWindow(request, app: app)
        let focused = try focusedElement(app: app, window: window)
        try bind(element, to: window, pid: request.pid)
        let geometry = try geometry(request, element: element)
        guard try bounds(window) == geometry.windowBounds else { throw Refusal.target }
        _ = try geometry.checkedLocation(geometry.location, current: geometry)
        if request.ownerWindow {
            guard CFEqual(focused, element) else { throw Refusal.target }
        }
        if typing {
            guard CFEqual(focused, element), request.ownerWindow || request.target == "ordinary",
                  let role = try attribute(element, kAXRoleAttribute) as? String,
                  [kAXTextFieldRole, kAXTextAreaRole].contains(role) else { throw Refusal.target }
        } else {
            let location: CGPoint
            if let clickGeometry {
                guard let eventLocation, try windowBounds(request) == geometry.windowBounds else { throw Refusal.target }
                location = try clickGeometry.checkedLocation(eventLocation, current: geometry)
            } else { location = geometry.location }
            var hit: AXUIElement?
            guard AXUIElementCopyElementAtPosition(app, Float(location.x), Float(location.y), &hit) == .success,
                  let hit else { throw Refusal.target }
            var candidate = hit, matched = false
            for _ in 0..<8 {
                if CFEqual(candidate, element) { matched = true; break }
                guard request.target == "scroll", let parent = try? attribute(candidate, kAXParentAttribute),
                      CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
                candidate = parent as! AXUIElement
            }
            guard matched else { throw Refusal.target }
            if let clickGeometry {
                // AX hit-testing can itself race a move or resize. Recheck its snapshot.
                _ = try clickGeometry.checkedLocation(location, current:
                    self.geometry(request, element: element))
                guard try bounds(window) == clickGeometry.windowBounds else { throw Refusal.target }
            }
        }
    }
    func bind(_ element: AXUIElement, to ancestor: AXUIElement, pid: pid_t) throws {
        var actual: pid_t = 0, candidate = element
        guard AXUIElementGetPid(element, &actual) == .success, actual == pid else { throw Refusal.target }
        for _ in 0..<32 {
            if (try? attribute(candidate, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole { throw Refusal.secure }
            if CFEqual(candidate, ancestor) { return }
            let parent = try attribute(candidate, kAXParentAttribute)
            guard CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw Refusal.target }
            candidate = parent as! AXUIElement
        }
        throw Refusal.target
    }
    func mouseReleaseFence(_ request: Request, application: NSRunningApplication,
                           launchDate: Date, window: AXUIElement, event: CGEvent) throws {
        // Only the already owned up may bypass saved geometry and hit-testing.
        guard event.type == .leftMouseUp, !application.isTerminated,
              application.processIdentifier == request.pid,
              let current = NSRunningApplication(processIdentifier: request.pid),
              current.launchDate == launchDate,
              event.getIntegerValueField(.mouseEventWindowUnderMousePointer) == Int64(request.window),
              event.getIntegerValueField(.mouseEventWindowUnderMousePointerThatCanHandleThisEvent) == Int64(request.window)
        else { throw Refusal.target }
        try validateTarget(request)
        let app = AXUIElementCreateApplication(request.pid)
        let selected = try selectedWindow(request, app: app)
        guard CFEqual(selected, window) else { throw Refusal.target }
        try bind(window, to: selected, pid: request.pid)
        _ = try focusedElement(app: app, window: selected)
        try checkPermission(.click)
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
              (0...1).contains(number.doubleValue) else { throw Refusal.target }
        return number.doubleValue
    }
    func performAX(_ path: InputPath, operation: Operation, request: Request,
                   element: AXUIElement) async throws -> String {
        var scroller: AXUIElement?, before: Double?, counter: Int?
        if path == .axPress {
            var names: CFArray?
            guard AXUIElementCopyActionNames(element, &names) == .success,
                  let actions = names as? [String], actions.contains(kAXPressAction) else { throw Refusal.target }
            counter = fixtureCounter(request, element: element)
        } else {
            let value = try attribute(element, kAXVerticalScrollBarAttribute)
            guard CFGetTypeID(value) == AXUIElementGetTypeID() else { throw Refusal.target }
            scroller = (value as! AXUIElement)
            guard let scroller, try attribute(scroller, kAXRoleAttribute) as? String == kAXScrollBarRole else { throw Refusal.target }
            try bind(scroller, to: element, pid: request.pid)
            var settable: DarwinBoolean = false
            guard AXUIElementIsAttributeSettable(scroller, kAXValueAttribute as CFString, &settable) == .success,
                  settable.boolValue else { throw Refusal.target }
            before = try scrollValue(scroller)
        }
        try checkPermission(operation)
        try fence(request, element: element, typing: false)
        if let scroller, let before {
            try bind(scroller, to: element, pid: request.pid)
            guard try scrollValue(scroller) == before else { throw Refusal.target }
            try fence(request, element: element, typing: false)
            guard AXUIElementSetAttributeValue(scroller, kAXValueAttribute as CFString,
                NSNumber(value: try nextScrollValue(before))) == .success else { throw Refusal.native }
        } else {
            guard AXUIElementPerformAction(element, kAXPressAction as CFString) == .success else { throw Refusal.native }
        }
        // Poll only the synthetic counter or numeric scrollbar. Never field text.
        for _ in 0..<4 {
            try await Task.sleep(nanoseconds: 50_000_000)
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
            guard #available(macOS 14.0, *) else { throw Refusal.permission }
            let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            guard let window = content.windows.first(where: { $0.windowID == request.window && $0.owningApplication?.processID == request.pid })
            else { throw Refusal.target }
            let filter = SCContentFilter(desktopIndependentWindow: window), config = SCStreamConfiguration()
            config.width = Int(filter.contentRect.width * CGFloat(filter.pointPixelScale))
            config.height = Int(filter.contentRect.height * CGFloat(filter.pointPixelScale))
            config.showsCursor = false; config.capturesAudio = false
            if #available(macOS 15.0, *) { config.captureMicrophone = false }
            let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            try fence(request, element: element, typing: false)
            guard CGPreflightScreenCaptureAccess() else { throw Refusal.permission }
            let bitmap = NSBitmapImageRep(cgImage: image)
            guard let png = bitmap.representation(using: .png, properties: [:]) else { throw Refusal.native }
            try png.write(to: URL(fileURLWithPath: path), options: .atomic)
            return "frame \(image.width)x\(image.height) window=\(request.window)"
        }
        if operation == .read {
            // Read only public metadata, never values, labels, selections or secure descendants.
            guard let role = try attribute(element, kAXRoleAttribute) as? String,
                  [kAXTextFieldRole, kAXTextAreaRole, kAXButtonRole, kAXScrollAreaRole].contains(role) else { throw Refusal.target }
            return "target=\(request.target) role=\(role)"
        }
        guard let role = try attribute(element, kAXRoleAttribute) as? String else { throw Refusal.target }
        let path = try inputPath(operation, role: role)
        if path == .axPress || path == .axScrollValue {
            return try await performAX(path, operation: operation, request: request, element: element)
        }
        let source = CGEventSource(stateID: .privateState)
        var events: [CGEvent] = []
        var clickGeometry: ClickGeometry?
        switch operation {
        case .click:
            let geometry = try geometry(request, element: element)
            if let receipt = try await performTextClick(request, element: element, geometry: geometry) { return receipt }
            clickGeometry = geometry
            events = try hidClickEvents(window: request.window, location: geometry.location,
                                          windowBounds: geometry.windowBounds, eventNumber: Int.random(in: 1...Int(Int32.max)))
        case .text(let text):
            let units = try textUnits(text)
            for down in [true, false] {
                // Keycode 0 is experimental; inertness is an owner-session gate.
                guard let event = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down) else { throw Refusal.native }
                units.withUnsafeBufferPointer { event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: $0.baseAddress) }
                events.append(event)
            }
        default: throw Refusal.arguments
        }
        guard CGPreflightPostEventAccess() else { throw Refusal.permission }
        let typing: Bool = { if case .text = operation { return true }; return false }()
        guard let admittedApp = NSRunningApplication(processIdentifier: request.pid),
              !admittedApp.isTerminated, let admittedLaunchDate = admittedApp.launchDate else { throw Refusal.target }
        let admittedWindow = try selectedWindow(request, app: AXUIElementCreateApplication(request.pid))
        try dispatchInputEvents(events, fence: { event in
            try fence(request, element: element, typing: typing,
                      clickGeometry: clickGeometry, eventLocation: event.location)
        }, releaseFence: { event in
            if typing {
                try fence(request, element: element, typing: true)
                try checkPermission(operation)
            } else {
                try mouseReleaseFence(request, application: admittedApp, launchDate: admittedLaunchDate,
                                      window: admittedWindow, event: event)
            }
        }, releaseAllowed: { CGPreflightPostEventAccess() }, post: {
            if typing { $0.postToPid(request.pid) } else { $0.post(tap: .cghidEventTap) }
        })
        try fence(request, element: element, typing: typing,
                  clickGeometry: clickGeometry, eventLocation: events.last?.location)
        return inputReceipt(path: typing ? "CGEventPIDText" : "CGEventHID")
    }
}

// TextEdit-only discovery, no capture, AX reads, window titles or automatic selection.
func ownerWindowChoices() throws -> [[String: Any]] {
    guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { throw Refusal.native }
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
