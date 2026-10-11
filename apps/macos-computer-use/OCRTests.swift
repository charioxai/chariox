import AppKit
import CoreText

@main struct OCRTests {
    static func main() throws {
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 20, width: 400, height: 100), scale: 1)
        let stamp = CaptureStamp(generation: "g1", surface: surface, protection: 1)
        let context = CGContext(data: nil, width: 400, height: 100, bitsPerComponent: 8, bytesPerRow: 1600,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.setFillColor(CGColor(gray: 1, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: 400, height: 100))
        let text = NSAttributedString(string: "PUBLIC CANARY", attributes: [
            .font: CTFontCreateWithName("Helvetica" as CFString, 36, nil),
            .foregroundColor: CGColor(gray: 0, alpha: 1)])
        context.textPosition = CGPoint(x: 20, y: 30); CTLineDraw(CTLineCreateWithAttributedString(text), context)
        let raw = context.makeImage()!
        let raster = try protectedRaster(raw, surface: surface, masks: [], wholeWindowMask: false)
        let publicFrame = ProtectedFrame(pixels: raster.pixels, stamp: stamp, masked: false)
        let visible = try VisionOCR().observe(publicFrame, expected: stamp, scrub: { $0 }, authority: {})
        precondition(visible.contains { $0.text.contains("CANARY") }, "real Vision could not read public synthetic canary")
        let black = try protectedRaster(raw, surface: surface, masks: [], wholeWindowMask: true)
        let protected = ProtectedFrame(pixels: black.pixels, stamp: stamp, masked: true)
        let hidden = try VisionOCR().observe(protected, expected: stamp, scrub: { $0 }, authority: {})
        precondition(hidden.isEmpty, "Vision leaked masked canary")
        let ocr = VisionOCR(recognize: { _ in [OCRCandidate(text: "vault-canary", confidence: 0.95,
            alternativeConfidence: 0.94, box: CGRect(x: 0.25, y: 0.2, width: 0.5, height: 0.3))] })
        let result = try ocr.observe(publicFrame, expected: stamp, scrub: { $0.replacingOccurrences(of: "vault-canary", with: "[redacted]") }, authority: {})
        precondition(result[0].text == "[redacted]" && result[0].ambiguous)
        precondition(abs(result[0].bounds.minY - 70) < 0.0001 && abs(result[0].bounds.height - 30) < 0.0001 && result[0].bounds.minX == 0 && result[0].bounds.width == 200)
        let stale = CaptureStamp(generation: "g2", surface: surface, protection: 1)
        expectRefusal { _ = try ocr.observe(publicFrame, expected: stale, scrub: { $0 }, authority: {}) }
        var checks = 0
        expectRefusal {
            _ = try ocr.observe(publicFrame, expected: stamp, scrub: { $0 }, authority: {
                checks += 1; if checks == 2 { throw Refusal.disabled }
            })
        }
        print("PASS real local Vision reads public synthetic canary and cannot read opaque protected pixels; OCR boxes, ambiguity, retired-value scrub, stale frame and publication authority checks")
    }
    static func expectRefusal(_ body: () throws -> Void) {
        do { try body(); preconditionFailure("stale OCR published") } catch {}
    }
}
