import AppKit

@main struct M4ProtectionTests {
    static func main() throws {
        let surface = NativeSurface(bounds: CGRect(x: -10, y: 20, width: 10, height: 10), scale: 2)
        let context = CGContext(data: nil, width: 20, height: 20, bitsPerComponent: 8,
            bytesPerRow: 80, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 20, height: 20))
        let canary = context.makeImage()!
        var refused = false
        do {
            _ = try protectedRaster(canary, surface: surface,
                masks: [CGRect(x: -5, y: 25, width: 0, height: 2)], wholeWindowMask: false)
        } catch { refused = refusalKind(error) == .secure }
        precondition(refused, "unproven private geometry published canary pixels")
        print("PASS unknown private-region geometry withholds pixels before encoding or OCR")
    }
}
