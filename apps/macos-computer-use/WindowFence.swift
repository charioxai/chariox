import CoreGraphics

struct WindowOwnership {
    let id: UInt32
    let pid: Int32
    let bounds: CGRect
    let alpha: Double
}

// Front-to-back WindowServer order, freshly sampled at each mutation boundary.
// The exact target must still exist in this snapshot with its bound geometry.
func checkWindowStack(_ windows: [WindowOwnership], request: Request,
                      point: CGPoint, windowBounds: CGRect) throws {
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
        if window.alpha > 0, window.bounds.contains(point) {
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
              let alpha = info[kCGWindowAlpha as String] as? Double else {
            throw tagged(.target, "window-stack-unknown")
        }
        return WindowOwnership(id: id, pid: pid, bounds: bounds, alpha: alpha)
    }
    try checkWindowStack(windows, request: request, point: point, windowBounds: windowBounds)
}
