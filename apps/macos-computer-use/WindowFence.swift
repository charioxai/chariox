import AppKit
import ApplicationServices
import CoreGraphics

struct WindowOwnership {
    let id: UInt32
    let pid: Int32
    let bounds: CGRect
    let alpha: Double
    let layer: Int
    let systemUI: Bool
    init(id: UInt32, pid: Int32, bounds: CGRect, alpha: Double, layer: Int = 0, systemUI: Bool = false) {
        self.id = id; self.pid = pid; self.bounds = bounds; self.alpha = alpha
        self.layer = layer; self.systemUI = systemUI
    }
}

// Front-to-back WindowServer order, freshly sampled at each mutation boundary.
// The exact target must still exist in this snapshot with its bound geometry.
func checkWindowStack(_ windows: [WindowOwnership], request: Request,
                      point: CGPoint, windowBounds: CGRect,
                      inputPassesThrough: (WindowOwnership) -> Bool = { _ in false }) throws {
    guard point.x.isFinite, point.y.isFinite, windowBounds.contains(point),
          windows.filter({ $0.id == request.window }).count == 1,
          let target = windows.first(where: { $0.id == request.window }),
          target.pid == request.pid, target.bounds == windowBounds, target.alpha > 0 else {
        throw tagged(.target, "stale-window-stack")
    }
    for window in windows {
        guard window.alpha.isFinite, (0...1).contains(window.alpha),
              [window.bounds.minX, window.bounds.minY, window.bounds.width, window.bounds.height].allSatisfy({ $0.isFinite }),
              window.bounds.width > 0, window.bounds.height > 0 else {
            throw tagged(.target, "window-stack-unknown")
        }
        if window.id == request.window { return }
        if window.layer < target.layer || window.alpha == 0 { continue }
        if window.bounds.contains(point) {
            // A system identity alone never proves input transparency.
            if window.systemUI, inputPassesThrough(window) { continue }
            throw tagged(.target, "foreign-window-cover")
        }
    }
    throw tagged(.target, "stale-window-stack")
}

func checkWindowOcclusion(_ request: Request, point: CGPoint, windowBounds: CGRect) throws {
    guard let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]],
          list.count <= 4096 else { throw tagged(.target, "window-stack-unavailable") }
    let windows = try list.map { info -> WindowOwnership in
        guard let id = info[kCGWindowNumber as String] as? UInt32,
              let pid = info[kCGWindowOwnerPID as String] as? Int32,
              let data = info[kCGWindowBounds as String] as? [String: Any],
              let bounds = CGRect(dictionaryRepresentation: data as CFDictionary),
              let alpha = info[kCGWindowAlpha as String] as? Double,
              let layer = info[kCGWindowLayer as String] as? Int else {
            throw tagged(.target, "window-stack-unknown")
        }
        return WindowOwnership(id: id, pid: pid, bounds: bounds, alpha: alpha, layer: layer,
                               systemUI: isVerifiedSystemUI(pid))
    }
    try checkWindowStack(windows, request: request, point: point, windowBounds: windowBounds,
                         inputPassesThrough: { _ in systemHitMatches(request, point: point, windowBounds: windowBounds) })
}

// Restrict passthrough evidence to running Apple system UI code. Ordinary app
// windows, including same-process covers, always retain the CG visibility fence.
private func isVerifiedSystemUI(_ pid: Int32) -> Bool {
    let identifiers = ["com.apple.dock", "com.apple.WindowServer", "com.apple.systemuiserver",
                       "com.apple.controlcenter", "com.apple.notificationcenterui"]
    guard let app = NSRunningApplication(processIdentifier: pid) else { return false }
    let identifier = app.bundleIdentifier ?? (app.executableURL?.lastPathComponent == "WindowServer" ? "com.apple.WindowServer" : "")
    guard identifiers.contains(identifier) else { return false }
    return runningCodeSatisfiesRequirement(pid: pid, requirement: "identifier \"\(identifier)\" and anchor apple")
}

private func systemHitMatches(_ request: Request, point: CGPoint, windowBounds: CGRect) -> Bool {
    // A fresh system-wide hit must land in this exact target window. An app-scoped
    // hit, unavailable AX, or a hit in another window cannot exempt system UI.
    let system = AXUIElementCreateSystemWide()
    guard AXUIElementSetMessagingTimeout(system, 0.5) == .success else { return false }
    var hit: AXUIElement?
    guard AXUIElementCopyElementAtPosition(system, Float(point.x), Float(point.y), &hit) == .success,
          let hit else { return false }
    var pid: pid_t = 0
    guard AXUIElementGetPid(hit, &pid) == .success, pid == request.pid else { return false }
    var candidate = hit
    for _ in 0..<32 {
        guard AXUIElementSetMessagingTimeout(candidate, 0.5) == .success else { return false }
        var role: CFTypeRef?
        guard AXUIElementCopyAttributeValue(candidate, kAXRoleAttribute as CFString, &role) == .success else { return false }
        if role as? String == kAXWindowRole {
            var position: CFTypeRef?, size: CFTypeRef?
            guard AXUIElementCopyAttributeValue(candidate, kAXPositionAttribute as CFString, &position) == .success,
                  AXUIElementCopyAttributeValue(candidate, kAXSizeAttribute as CFString, &size) == .success,
                  let position, let size, CFGetTypeID(position) == AXValueGetTypeID(),
                  CFGetTypeID(size) == AXValueGetTypeID() else { return false }
            var origin = CGPoint.zero, extent = CGSize.zero
            guard AXValueGetValue(position as! AXValue, .cgPoint, &origin),
                  AXValueGetValue(size as! AXValue, .cgSize, &extent) else { return false }
            return CGRect(origin: origin, size: extent) == windowBounds
        }
        var parent: CFTypeRef?
        guard AXUIElementCopyAttributeValue(candidate, kAXParentAttribute as CFString, &parent) == .success,
              let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { return false }
        candidate = parent as! AXUIElement
    }
    return false
}
