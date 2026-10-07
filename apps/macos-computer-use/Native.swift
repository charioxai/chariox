import AppKit
import ApplicationServices
import Carbon
import ScreenCaptureKit
import Security

struct MacSource: NativeSource {
    func validateFixture(_ request: Request) throws { try fixture(request) }
    func fixture(_ request: Request) throws {
        let expected = Bundle.main.bundleURL.deletingLastPathComponent()
            .appendingPathComponent("Chariox Computer Fixture.app/Contents/MacOS/fixture").resolvingSymlinksInPath()
        guard let app = NSRunningApplication(processIdentifier: request.pid), !app.isTerminated,
              app.bundleIdentifier == "ai.chariox.computer-fixture",
              app.executableURL?.resolvingSymlinksInPath() == expected else { throw Refusal.target }
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(expected as CFURL, [], &code) == errSecSuccess,
              let code, SecStaticCodeCheckValidity(code, [], nil) == errSecSuccess else { throw Refusal.target }
        guard !IsSecureEventInputEnabled() else { throw Refusal.secure }
        guard let info = CGWindowListCopyWindowInfo([.optionIncludingWindow], request.window) as? [[String: Any]],
              info.count == 1, info[0][kCGWindowOwnerPID as String] as? Int == Int(request.pid),
              info[0][kCGWindowIsOnscreen as String] as? Bool == true else { throw Refusal.target }
    }
    func attribute(_ element: AXUIElement, _ name: String) throws -> CFTypeRef {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success,
              let value else { throw Refusal.target }
        return value
    }
    func target(_ request: Request) throws -> AXUIElement {
        try fixture(request)
        guard AXIsProcessTrustedWithOptions([kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: false] as CFDictionary)
        else { throw Refusal.permission }
        let app = AXUIElementCreateApplication(request.pid)
        guard AXUIElementSetMessagingTimeout(app, 0.5) == .success else { throw Refusal.target }
        guard let windows = try attribute(app, kAXWindowsAttribute) as? [AXUIElement], windows.count == 1,
              let window = windows.first,
              try attribute(window, kAXTitleAttribute) as? String == "Chariox public fixture" else { throw Refusal.target }
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
    func point(_ target: AXUIElement) throws -> CGPoint {
        let position = try attribute(target, kAXPositionAttribute), size = try attribute(target, kAXSizeAttribute)
        guard CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw Refusal.target }
        var origin = CGPoint.zero, extent = CGSize.zero
        guard AXValueGetValue(position as! AXValue, .cgPoint, &origin),
              AXValueGetValue(size as! AXValue, .cgSize, &extent), extent.width > 0, extent.height > 0 else { throw Refusal.target }
        return CGPoint(x: origin.x + extent.width / 2, y: origin.y + extent.height / 2)
    }
    func fence(_ request: Request, element: AXUIElement, typing: Bool) throws {
        try fixture(request)
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == request.pid else { throw Refusal.target }
        let app = AXUIElementCreateApplication(request.pid)
        if typing {
            let focused = try attribute(app, kAXFocusedUIElementAttribute)
            guard CFGetTypeID(focused) == AXUIElementGetTypeID(),
                  CFEqual(focused, element), request.target == "ordinary" else { throw Refusal.target }
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
        try fixture(request)
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
            try fixture(request)
            guard CGPreflightScreenCaptureAccess() else { throw Refusal.permission }
            let bitmap = NSBitmapImageRep(cgImage: image)
            guard let png = bitmap.representation(using: .png, properties: [:]) else { throw Refusal.native }
            try png.write(to: URL(fileURLWithPath: path), options: .atomic)
            return "frame \(image.width)x\(image.height) window=\(request.window)"
        }
        let element = try target(request)
        if operation == .read {
            // Read only public metadata, never values, labels, selections or secure descendants.
            guard let role = try attribute(element, kAXRoleAttribute) as? String,
                  [kAXTextFieldRole, kAXButtonRole, kAXScrollAreaRole].contains(role) else { throw Refusal.target }
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
                // Keycode 0 is experimental, fixture-only; inertness is an owner-session gate.
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
            if let release, (try? fixture(request)) != nil, CGPreflightPostEventAccess() {
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
