import Foundation

struct DamageRect: Equatable {
    let x: Int, y: Int, width: Int, height: Int
}
struct CaptureStamp: Equatable {
    let generation: String
    let surface: NativeSurface
    let protection: UInt64
}
struct CapturePublication {
    let sequence: UInt64
    let base: UInt64?
    let exact: Bool
    let rects: [DamageRect]
}

// One protected RGBA base per attachment, bounded before allocation/retention.
// No raw capture or AX metadata enters this cache. A stale/unknown base always
// bootstraps a full exact image. Explicit repair also sends the full exact image.
final class CaptureDamage {
    private var previous: [UInt8]?
    private var stamp: CaptureStamp?
    private var sequence: UInt64 = 0
    func invalidate() { previous = nil; stamp = nil }
    func next(_ pixels: [UInt8], stamp: CaptureStamp, after: UInt64?, repair: Bool) throws -> CapturePublication {
        let width = stamp.surface.width, height = stamp.surface.height
        guard width > 0, height > 0, width <= 4096, height <= 4096,
              width * height <= 4 * 1024 * 1024, pixels.count == width * height * 4,
              sequence < UInt64.max else { throw tagged(.native, "capture-budget") }
        let full = DamageRect(x: 0, y: 0, width: width, height: height)
        var rects: [DamageRect] = []
        let base = sequence
        let exact = repair || self.stamp != stamp || previous == nil || after != sequence
        if exact { rects = [full] }
        else if let previous {
            // Compare bytes, not lossy perceptual hashes. Empty damage advances
            // no viewer state; the next base remains the acknowledged sequence.
            for y in stride(from: 0, to: height, by: 64) {
                for x in stride(from: 0, to: width, by: 64) {
                    let w = min(64, width - x), h = min(64, height - y)
                    var changed = false
                    for row in y..<(y + h) {
                        let start = (row * width + x) * 4, end = start + w * 4
                        if pixels[start..<end] != previous[start..<end] { changed = true; break }
                    }
                    if changed { rects.append(DamageRect(x: x, y: y, width: w, height: h)) }
                }
            }
        }
        if !rects.isEmpty { sequence += 1 }
        self.previous = pixels; self.stamp = stamp
        return CapturePublication(sequence: sequence, base: exact ? nil : base, exact: exact, rects: rects)
    }
}
