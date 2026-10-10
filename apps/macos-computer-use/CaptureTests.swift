import AppKit

@main struct CaptureTests {
    static func main() throws {
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 20, width: 65, height: 40), scale: 2)
        var stamp = CaptureStamp(generation: "g1", surface: surface, protection: 1)
        let cache = CaptureDamage()
        let full = DamageRect(x: 0, y: 0, width: 130, height: 80)
        var pixels = [UInt8](repeating: 0, count: 130 * 80 * 4)
        let first = try cache.next(pixels, stamp: stamp, after: nil, repair: false)
        precondition(first.exact && first.base == nil && first.rects == [full] && first.sequence == 1)
        let quiet = try cache.next(pixels, stamp: stamp, after: 1, repair: false)
        precondition(!quiet.exact && quiet.rects.isEmpty && quiet.sequence == 1 && quiet.base == 1)
        pixels[(79 * 130 + 129) * 4] = 255
        let damage = try cache.next(pixels, stamp: stamp, after: 1, repair: false)
        precondition(!damage.exact && damage.sequence == 2 && damage.base == 1)
        precondition(damage.rects == [DamageRect(x: 128, y: 64, width: 2, height: 16)])
        let lost = try cache.next(pixels, stamp: stamp, after: 1, repair: false)
        precondition(lost.exact && lost.base == nil && lost.rects == [full])
        let repair = try cache.next(pixels, stamp: stamp, after: lost.sequence, repair: true)
        precondition(repair.exact && repair.rects == [full])
        stamp = CaptureStamp(generation: "g1", surface: surface, protection: 2)
        let protected = try cache.next(pixels, stamp: stamp, after: repair.sequence, repair: false)
        precondition(protected.exact && protected.base == nil)
        cache.invalidate()
        let fresh = try cache.next(pixels, stamp: stamp, after: protected.sequence, repair: false)
        precondition(fresh.exact)
        stamp = CaptureStamp(generation: "g2", surface: surface, protection: 2)
        let newEpoch = try cache.next(pixels, stamp: stamp, after: fresh.sequence, repair: false)
        precondition(newEpoch.exact)
        do { _ = try cache.next([0], stamp: stamp, after: nil, repair: false); preconditionFailure("invalid raster admitted") }
        catch { precondition(refusalKind(error) == .native) }
        print("PASS exact bootstrap, byte-exact edge damage, quiet frame, lost-base repair, explicit repair, protection flush and generation reset")
        print("PASS bounded protected RGBA cache and negative-origin 2x surface")
        let cg = CGContext(data: nil, width: surface.width, height: surface.height, bitsPerComponent: 8,
            bytesPerRow: surface.width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)!
        cg.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
        cg.fill(CGRect(x: 0, y: 0, width: surface.width, height: surface.height))
        let canary = cg.makeImage()!
        let mask = CGRect(x: -90, y: 30, width: 10, height: 10)
        let maskedRaster = try protectedRaster(canary, surface: surface, masks: [mask], wholeWindowMask: false)
        // RGBA rows use the image top-left. Quartz y=30 maps to pixel y=20.
        let inside = (30 * surface.width + 30) * 4, outside = (5 * surface.width + 5) * 4
        precondition(Array(maskedRaster.pixels[inside..<(inside + 4)]) == [0, 0, 0, 255])
        let baseline = try protectedRaster(canary, surface: surface, masks: [], wholeWindowMask: false)
        precondition(Array(maskedRaster.pixels[outside..<(outside + 4)]) == Array(baseline.pixels[outside..<(outside + 4)]))
        let opaque = try protectedRaster(canary, surface: surface, masks: [], wholeWindowMask: true)
        precondition(stride(from: 0, to: opaque.pixels.count, by: 4).allSatisfy { Array(opaque.pixels[$0..<($0 + 4)]) == [0, 0, 0, 255] })
        print("PASS secure-region mask is opaque at 2x negative origins; whole-window protection has no canary pixels")
    }
}
