import AppKit
import ApplicationServices
import Carbon
import ScreenCaptureKit
import Security

struct MacSource: NativeSource {
    func validateTarget(_ request: Request) throws {
        guard let app = NSRunningApplication(processIdentifier: request.pid), !app.isTerminated else { throw Refusal.target }
        if !request.ownerWindow {
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
        guard AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: false] as CFDictionary)
        else { throw Refusal.permission }
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
    func point(_ target: AXUIElement) throws -> CGPoint {
        let rect = try bounds(target)
        return CGPoint(x: rect.midX, y: rect.midY)
    }
    func selectedWindow(_ request: Request, app: AXUIElement) throws -> AXUIElement {
        let rect = try windowBounds(request)
        guard let windows = try attribute(app, kAXWindowsAttribute) as? [AXUIElement], windows.count <= 32 else { throw Refusal.target }
        // Public AX exposes geometry, not a CG window ID. Refuse ambiguous matches.
        let matches = try windows.filter { try bounds($0) == rect }
        guard matches.count == 1, let window = matches.first,
              CFEqual(try attribute(app, kAXFocusedWindowAttribute), window) else { throw Refusal.target }
        // Another CG window at the same bounds would make the AX binding ambiguous.
        guard let all = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] else { throw Refusal.target }
        let same = all.filter {
            guard $0[kCGWindowOwnerPID as String] as? Int == Int(request.pid),
                  let value = $0[kCGWindowBounds as String] as? [String: Any] else { return false }
            return CGRect(dictionaryRepresentation: value as CFDictionary) == rect
        }
        try checkWindowBinding(pid: request.pid,
                               ownerPID: Int32(same.first?[kCGWindowOwnerPID as String] as? Int ?? 0),
                               frontmostPID: NSWorkspace.shared.frontmostApplication?.processIdentifier,
                               axMatches: matches.count, cgMatches: same.count,
                               focused: CFEqual(try attribute(app, kAXFocusedWindowAttribute), window))
        return window
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
    func fence(_ request: Request, element: AXUIElement, typing: Bool) throws {
        try validateTarget(request)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == request.pid else { throw Refusal.target }
        let app = AXUIElementCreateApplication(request.pid)
        let window = try selectedWindow(request, app: app)
        let focused = try focusedElement(app: app, window: window)
        if request.ownerWindow {
            guard CFEqual(focused, element) else { throw Refusal.target }
            var pid: pid_t = 0
            guard AXUIElementGetPid(element, &pid) == .success, pid == request.pid else { throw Refusal.target }
            guard try bounds(window).contains(try bounds(element)) else { throw Refusal.target }
        }
        if typing {
            guard CFEqual(focused, element), request.ownerWindow || request.target == "ordinary",
                  let role = try attribute(element, kAXRoleAttribute) as? String,
                  [kAXTextFieldRole, kAXTextAreaRole].contains(role) else { throw Refusal.target }
        } else {
            let location = try point(element)
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
        }
    }
    func perform(_ operation: Operation, request: Request) async throws -> String {
        try validateTarget(request)
        let element = try target(request)
        try fence(request, element: element, typing: false)
        if case .capture(let path) = operation {
            guard #available(macOS 14.0, *), CGPreflightScreenCaptureAccess() else { throw Refusal.permission }
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
        let source = CGEventSource(stateID: .privateState)
        var events: [CGEvent] = []
        switch operation {
        case .click:
            for type in [CGEventType.leftMouseDown, .leftMouseUp] {
                guard let event = CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: try point(element), mouseButton: .left)
                else { throw Refusal.native }; events.append(event)
            }
        case .scroll:
            guard let event = CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 1, wheel1: -24, wheel2: 0, wheel3: 0)
            else { throw Refusal.native }; event.location = try point(element); events.append(event)
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
        var release: CGEvent?
        defer {
            // Best effort for a fence failure between the paired events; fatal death is unproven.
            if let release, (try? fence(request, element: element, typing: typing)) != nil, CGPreflightPostEventAccess() {
                release.flags = []; release.postToPid(request.pid)
            }
        }
        for event in events {
            try fence(request, element: element, typing: typing)
            if event.type == .leftMouseDown || event.type == .keyDown { release = events.last }
            event.flags = []; event.postToPid(request.pid)
            if event.type == .leftMouseUp || event.type == .keyUp { release = nil }
        }
        try fence(request, element: element, typing: typing)
        return "dispatched; application completion unproven"
    }
}

// Owner-only discovery, no capture, AX reads, window titles or automatic selection.
func ownerWindowChoices() throws -> [[String: Any]] {
    guard let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else { throw Refusal.native }
    return windows.compactMap { info in
        guard let pid = info[kCGWindowOwnerPID as String] as? Int,
              let id = info[kCGWindowNumber as String] as? UInt32,
              info[kCGWindowLayer as String] as? Int == 0,
              let app = NSRunningApplication(processIdentifier: Int32(pid)),
              let bundle = app.bundleIdentifier else { return nil }
        return ["pid": pid, "windowID": id, "bundle": bundle]
    }
}
