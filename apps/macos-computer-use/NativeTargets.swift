import AppKit

@MainActor final class NativeTargets {
    private let registry = TargetRegistry<AXUIElement>()
    private var observer: AXObserver?
    private var watched: [(AXUIElement, CFString)] = []
    deinit { if let observer { CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes) } }
    func invalidate() { registry.invalidate() }
    func stop() {
        if let observer {
            for (node, name) in watched { AXObserverRemoveNotification(observer, node, name) }
            CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
        }
        observer = nil; watched.removeAll(); invalidate()
    }
    var revision: UInt64 { registry.currentRevision }
    private func bind(_ scope: Request, stamp: CaptureStamp, observer: String) throws -> TargetContext {
        guard let app = NSRunningApplication(processIdentifier: scope.pid), !app.isTerminated,
              let launched = app.launchDate else { throw tagged(.target, "target-process-identity-unknown") }
        return TargetContext(processStart: "\(scope.pid):\(launched.timeIntervalSince1970)", stamp: stamp, observer: observer)
    }
    func observe(_ scope: Request, stamp: CaptureStamp, observer: String, source: MacSource,
                 scrub: (String) throws -> String) throws -> [ObservedTarget] {
        stop()
        var completed = false
        defer { if !completed { stop() } }
        try source.checkPermission(.click)
        let target = try source.target(scope)
        try source.fence(scope, element: target, typing: false)
        let bound = try bind(scope, stamp: stamp, observer: observer)
        let app = AXUIElementCreateApplication(scope.pid)
        let root = try source.selectedWindow(scope, app: app)
        var created: AXObserver?
        guard AXObserverCreate(scope.pid, { _, _, _, pointer in
            guard let pointer else { return }
            MainActor.assumeIsolated { Unmanaged<NativeTargets>.fromOpaque(pointer).takeUnretainedValue().invalidate() }
        }, &created) == .success, let created else { throw tagged(.target, "target-observer-unavailable") }
        self.observer = created
        CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(created), .commonModes)
        let reader = TargetReader<AXUIElement>(secure: { element in
            guard AXUIElementSetMessagingTimeout(element, 0.05) == .success else { throw Refusal.target }
            return try captureElementIsSecure { self.read(element, kAXSubroleAttribute) }
        }, metadata: { element in
            for name in [kAXUIElementDestroyedNotification, kAXMovedNotification, kAXResizedNotification,
                         kAXValueChangedNotification, kAXFocusedUIElementChangedNotification] {
                let status = AXObserverAddNotification(created, element, name as CFString, Unmanaged.passUnretained(self).toOpaque())
                guard status == .success || status == .notificationUnsupported else { throw tagged(.target, "target-observer-registration") }
                if status == .success { self.watched.append((element, name as CFString)) }
            }
            return try self.metadata(element, source: source)
        }, children: { element in
            let (status, value) = self.read(element, kAXChildrenAttribute)
            if status == .attributeUnsupported || status == .noValue { return [] }
            guard status == .success, let children = value as? [AXUIElement] else { throw tagged(.target, "target-children-unknown") }
            return children
        })
        let result = try registry.observe(root: root, context: bound, reader: reader, scrub: scrub)
        try source.fence(scope, element: target, typing: false)
        guard try bind(scope, stamp: stamp, observer: observer) == bound, try nativeSurface(scope) == stamp.surface else { throw Refusal.target }
        completed = true
        return result
    }
    func press(_ ref: String, revision: UInt64, observer: String, scope: Request, stamp: CaptureStamp, source: MacSource) async throws {
        guard registry.currentRevision == revision else { throw tagged(.target, "target-ref-stale") }
        let bound = try bind(scope, stamp: stamp, observer: observer)
        let consumed = try registry.consume(ref, context: bound) { node, expected in
            guard expected.enabled, expected.role == kAXButtonRole, expected.actions.contains(kAXPressAction) else { return false }
            return try self.validate(node, expected: expected, scope: scope, stamp: stamp, source: source)
        }
        // Consume before dispatch so a failed post-effect receipt cannot replay.
        let node = consumed.node
        try consumed.perform(validate: { node, expected in
            try source.checkPermission(.click)
            try source.fence(scope, element: node, typing: false)
            guard try validate(node, expected: expected, scope: scope, stamp: stamp, source: source) else { throw Refusal.target }
        }, mutation: { node in
            guard AXUIElementPerformAction(node, kAXPressAction as CFString) == .success else { throw tagged(.native, "press-mutation-failed") }
        })
        try source.authority()
        try source.checkPermission(.click)
        try source.fence(scope, element: node, typing: false)
    }
    private func validate(_ node: AXUIElement, expected: TargetMetadata, scope: Request, stamp: CaptureStamp, source: MacSource) throws -> Bool {
        try source.checkPermission(.click)
        try source.fence(scope, element: node, typing: false)
        guard try nativeSurface(scope) == stamp.surface else { return false }
        let root = try source.selectedWindow(scope, app: AXUIElementCreateApplication(scope.pid))
        var ancestor = node
        for _ in 0..<32 {
            if try captureElementIsSecure(read: { self.read(ancestor, kAXSubroleAttribute) }) { return false }
            if CFEqual(ancestor, root) { return try self.metadata(node, source: source) == expected }
            let parent = try source.attribute(ancestor, kAXParentAttribute)
            guard CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw Refusal.target }
            ancestor = parent as! AXUIElement
        }
        return false
    }
    private func read(_ node: AXUIElement, _ name: String) -> (AXError, CFTypeRef?) {
        var value: CFTypeRef?
        let status = AXUIElementCopyAttributeValue(node, name as CFString, &value)
        return (status, value)
    }
    private func optional<T>(_ node: AXUIElement, _ name: String, as type: T.Type) throws -> T? {
        let (status, value) = read(node, name)
        if status == .attributeUnsupported || status == .noValue { return nil }
        guard status == .success, let result = value as? T else { throw tagged(.target, "target-metadata-unknown") }
        return result
    }
    private func metadata(_ node: AXUIElement, source: MacSource) throws -> TargetMetadata {
        var names: CFArray?
        guard AXUIElementCopyActionNames(node, &names) == .success, let actions = names as? [String],
              let role = try optional(node, kAXRoleAttribute, as: String.self) else { throw Refusal.target }
        // Never read AXValue, descriptions, selections or arbitrary attributes.
        let editable = [kAXTextFieldRole, kAXTextAreaRole].contains(role)
        return try TargetMetadata(role: role, subrole: optional(node, kAXSubroleAttribute, as: String.self),
            label: editable ? nil : optional(node, kAXTitleAttribute, as: String.self), bounds: source.bounds(node),
            enabled: optional(node, kAXEnabledAttribute, as: Bool.self) ?? false,
            focused: optional(node, kAXFocusedAttribute, as: Bool.self) ?? false, actions: actions)
    }
}
