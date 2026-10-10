import AppKit
import ScreenCaptureKit

@MainActor final class PairedCapture {
    private var policy: Data?
    private var allowed = false
    private var wholeWindowMask = true
    private var protection: UInt64 = 0
    private let damage = CaptureDamage()

    func protect(_ value: [String: Any]) {
        // Retain only a policy comparison token in memory, never in evidence.
        let next = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        if next == policy { return }
        damage.invalidate(); protection &+= 1; policy = next
        allowed = value["unknown"] as? Bool == false
        // Existing Linux behavior masks all native pixels when registry targets
        // or values exist. Unknown policy withholds capture before the OS call.
        wholeWindowMask = !(value["values"] as? [Any] ?? []).isEmpty || !(value["targets"] as? [Any] ?? []).isEmpty
    }
    func clear() { damage.invalidate(); policy = nil; allowed = false; wholeWindowMask = true }

    func screenshot(_ scope: Request, surface: NativeSurface, generation: String,
                    authority: @escaping () throws -> Void) async throws -> [String: Any] {
        let frame = try await capture(scope, surface: surface, generation: generation,
                                      after: nil, repair: true, authority: authority)
        return ["mime_type": "image/png", "data_base64": frame.png.base64EncodedString(),
                "width": surface.width, "height": surface.height, "protected": frame.masked,
                "surface_id": "macos-seat", "generation": generation]
    }

    // Private source/attachment seam. Public display registration and encrypted
    // viewer delivery must reuse the kernel display service in the next M2 slice.
    func capture(_ scope: Request, surface: NativeSurface, generation: String,
                 after: UInt64?, repair: Bool, authority: @escaping () throws -> Void)
        async throws -> (png: Data, publication: CapturePublication, masked: Bool) {
        guard allowed, !scope.ownerWindow else { throw tagged(.secure, "capture-protection-unproven") }
        guard #available(macOS 14.0, *), surface.width * surface.height <= 4 * 1024 * 1024 else {
            throw tagged(.native, "capture-budget")
        }
        let source = MacSource(authority: authority)
        let element = try source.target(scope)
        try source.checkPermission(.capture("in-memory"))
        try source.fence(scope, element: element, typing: false)
        guard try nativeSurface(scope) == surface else { throw tagged(.target, "surface-geometry-changed") }
        let revision = protection
        let masks = try secureRegions(source, scope: scope)
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        try authority()
        guard let window = content.windows.first(where: { $0.windowID == scope.window && $0.owningApplication?.processID == scope.pid }) else {
            throw tagged(.target, "capture-window-unavailable")
        }
        let filter = SCContentFilter(desktopIndependentWindow: window)
        guard filter.contentRect.size == surface.bounds.size, CGFloat(filter.pointPixelScale) == surface.scale else {
            throw tagged(.target, "capture-transform-changed")
        }
        let config = SCStreamConfiguration()
        config.width = surface.width; config.height = surface.height
        config.showsCursor = false; config.capturesAudio = false
        if #available(macOS 15.0, *) { config.captureMicrophone = false }
        let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
        try source.checkPermission(.capture("in-memory"))
        try source.fence(scope, element: element, typing: false)
        guard revision == protection, allowed, try nativeSurface(scope) == surface,
              try secureRegions(source, scope: scope) == masks,
              image.width == surface.width, image.height == surface.height else {
            damage.invalidate(); throw tagged(.secure, "capture-protection-changed")
        }
        let (protectedImage, pixels) = try protectedRaster(image, surface: surface,
                                                         masks: masks, wholeWindowMask: wholeWindowMask)
        // Mask before PNG encoding or damage comparison. No raw frame is written.
        guard let png = NSBitmapImageRep(cgImage: protectedImage).representation(using: .png, properties: [:]),
              png.count <= 512 * 1024 else { throw tagged(.native, "capture-encoded-budget") }
        try authority()
        let stamp = CaptureStamp(generation: generation, surface: surface, protection: revision)
        let publication = try damage.next(pixels, stamp: stamp, after: after, repair: repair)
        return (png, publication, wholeWindowMask || !masks.isEmpty)
    }

    private func secureRegions(_ source: MacSource, scope: Request) throws -> [CGRect] {
        let app = AXUIElementCreateApplication(scope.pid)
        var pending = [try source.selectedWindow(scope, app: app)], masks: [CGRect] = [], count = 0
        while let element = pending.popLast() {
            count += 1
            guard count <= 128 else { throw tagged(.secure, "capture-ax-coverage-budget") }
            // Only the signed public fixture is admitted at this checkpoint.
            // Missing child coverage refuses instead of attempting OCR on raw pixels.
            if (try? source.attribute(element, kAXSubroleAttribute) as? String) == kAXSecureTextFieldSubrole {
                masks.append(try source.bounds(element)); continue
            }
            var value: CFTypeRef?
            let status = AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value)
            if status == .attributeUnsupported || status == .noValue { continue }
            guard status == .success, let children = value as? [AXUIElement], children.count <= 128 - count else {
                throw tagged(.secure, "capture-ax-coverage-unknown")
            }
            pending += children
        }
        return masks.sorted { $0.minY == $1.minY ? $0.minX < $1.minX : $0.minY < $1.minY }
    }
}
