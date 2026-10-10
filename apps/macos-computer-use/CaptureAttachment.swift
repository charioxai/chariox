import AppKit

struct ProtectedFrame {
    let pixels: [UInt8]
    let stamp: CaptureStamp
    let masked: Bool
}

// Private helper source contract only. The kernel must supply the attachment
// owner after caller admission; these methods are never a public tool route.
@MainActor final class CaptureAttachment {
    private final class Viewer {
        let owner: String, generation: String
        let damage = CaptureDamage()
        var busy = false
        var expires: TimeInterval
        init(owner: String, generation: String, now: TimeInterval) {
            self.owner = owner; self.generation = generation; expires = now + 60
        }
    }
    private var viewers: [String: Viewer] = [:]
    private var revision: UInt64 = 0
    private let now: () -> TimeInterval
    init(now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) { self.now = now }

    func subscribe(owner: String, generation: String) throws -> String {
        viewers = viewers.filter { $0.value.expires > now() }
        guard !owner.isEmpty, owner.utf8.count <= 256, !generation.isEmpty,
              viewers.count < 2 else { throw tagged(.arguments, "capture-subscription-budget") }
        let id = UUID().uuidString
        viewers[id] = Viewer(owner: owner, generation: generation, now: now())
        return id
    }
    func invalidate() {
        revision &+= 1
        for viewer in viewers.values { viewer.damage.invalidate() }
    }
    func close(id: String, owner: String) {
        if viewers[id]?.owner == owner { viewers.removeValue(forKey: id) }
    }
    func close() { invalidate(); viewers.removeAll() }

    // One outstanding credit per viewer. No capture/encode work before admission.
    // Expired/closed/protection-changed work cannot commit after an async capture.
    func sample(id: String, owner: String, after: UInt64?, repair: Bool,
                authority: () throws -> Void, capture: () async throws -> ProtectedFrame)
        async throws -> [String: Any]? {
        guard let viewer = viewers[id], viewer.owner == owner, viewer.expires > now(), !viewer.busy else {
            throw tagged(.disabled, "capture-subscription-unavailable")
        }
        try authority()
        viewer.busy = true
        defer { viewer.busy = false }
        let revision = self.revision
        do {
            let frame = try await capture()
            try authority()
            guard viewers[id] === viewer, viewer.expires > now(), revision == self.revision,
                  viewer.generation == frame.stamp.generation else {
                throw tagged(.secure, "capture-subscription-changed")
            }
            let publication = try viewer.damage.next(frame.pixels, stamp: frame.stamp, after: after, repair: repair)
            viewer.expires = now() + 60
            if publication.rects.isEmpty { return nil }
            var packet: [String: Any] = ["subscription_id": id, "surface_id": "macos-seat",
                "generation": frame.stamp.generation, "sequence": publication.sequence,
                "width": frame.stamp.surface.width, "height": frame.stamp.surface.height,
                "device_scale_factor": frame.stamp.surface.scale, "colour": "srgb",
                "protection_revision": frame.stamp.protection, "protected": frame.masked]
            if publication.exact {
                packet["kind"] = "png"
                packet["data_base64"] = try encodeProtectedPNG(frame, rect: publication.rects[0]).base64EncodedString()
            } else {
                // Bound tiny-tile overhead as well as encoded pixels. Large damage
                // falls back to a full exact base, retaining the same sequence.
                var tiles: [[String: Any]] = []
                var bytes = 0
                if publication.rects.count <= 260 {
                    for rect in publication.rects {
                        let png = try encodeProtectedPNG(frame, rect: rect)
                        bytes += png.count
                        if bytes > 32 * 1024 { break }
                        tiles.append(["x": rect.x, "y": rect.y, "width": rect.width,
                                      "height": rect.height, "data_base64": png.base64EncodedString()])
                    }
                }
                if tiles.count == publication.rects.count {
                    packet["kind"] = "tiles"; packet["base_sequence"] = publication.base; packet["tiles"] = tiles
                } else {
                    packet["kind"] = "png"
                    let surface = frame.stamp.surface
                    packet["data_base64"] = try encodeProtectedPNG(frame,
                        rect: DamageRect(x: 0, y: 0, width: surface.width, height: surface.height)).base64EncodedString()
                }
            }
            // Reserve encrypted/base64 envelope overhead within Link's 1 MiB cap.
            guard try JSONSerialization.data(withJSONObject: packet).count <= 700 * 1024 else {
                throw tagged(.native, "capture-packet-budget")
            }
            try authority()
            guard viewers[id] === viewer, viewer.expires > now(), revision == self.revision else {
                throw tagged(.secure, "capture-subscription-changed")
            }
            return packet
        } catch {
            // Encoding/authority failure did not deliver a base. Next credit repairs.
            viewer.damage.invalidate()
            throw error
        }
    }
}

func encodeProtectedPNG(_ frame: ProtectedFrame, rect: DamageRect) throws -> Data {
    let surface = frame.stamp.surface
    guard surface.width > 0, surface.height > 0, surface.width * surface.height <= 4 * 1024 * 1024,
          frame.pixels.count == surface.width * surface.height * 4,
          rect.x >= 0, rect.y >= 0, rect.width > 0, rect.height > 0,
          rect.x + rect.width <= surface.width, rect.y + rect.height <= surface.height else {
        throw tagged(.native, "capture-raster-budget")
    }
    var pixels = [UInt8](repeating: 0, count: rect.width * rect.height * 4)
    for row in 0..<rect.height {
        let start = ((rect.y + row) * surface.width + rect.x) * 4
        pixels.replaceSubrange((row * rect.width * 4)..<((row + 1) * rect.width * 4),
                               with: frame.pixels[start..<(start + rect.width * 4)])
    }
    return try pixels.withUnsafeMutableBytes { data in
        guard let context = CGContext(data: data.baseAddress, width: rect.width, height: rect.height,
            bitsPerComponent: 8, bytesPerRow: rect.width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
            let image = context.makeImage(),
            let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]),
            png.count <= 512 * 1024 else { throw tagged(.native, "capture-encoded-budget") }
        return png
    }
}
