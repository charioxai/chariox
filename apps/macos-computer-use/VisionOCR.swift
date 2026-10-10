import AppKit
import Vision

struct OCRCandidate {
    let text: String
    let confidence: Float
    let alternativeConfidence: Float?
    let box: CGRect
}
struct ObservedText {
    let text: String
    let confidence: Float
    let bounds: CGRect
    let ambiguous: Bool
}
// No raw CGImage entry point. The only image accepted here is reconstructed
// from the capture pipeline's immutable protected RGBA frame, never OS pixels.
struct VisionOCR {
    var recognize: (CGImage) throws -> [OCRCandidate] = { image in
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        request.recognitionLanguages = ["en-US"]
        try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
        guard let results = request.results, results.count <= 128 else { throw tagged(.target, "ocr-budget") }
        return results.compactMap { observation in
            let candidates = observation.topCandidates(2)
            guard let first = candidates.first else { return nil }
            return OCRCandidate(text: first.string, confidence: first.confidence,
                alternativeConfidence: candidates.dropFirst().first?.confidence, box: observation.boundingBox)
        }
    }
    func observe(_ frame: ProtectedFrame, expected: CaptureStamp, scrub: (String) throws -> String,
                 authority: () throws -> Void) throws -> [ObservedText] {
        try authority()
        let surface = frame.stamp.surface
        guard frame.stamp == expected, surface.width > 0, surface.height > 0,
              surface.width * surface.height <= 4 * 1024 * 1024,
              frame.pixels.count == surface.width * surface.height * 4 else { throw tagged(.target, "ocr-frame-stale") }
        let data = Data(frame.pixels)
        guard let provider = CGDataProvider(data: data as CFData),
              let image = CGImage(width: surface.width, height: surface.height, bitsPerComponent: 8,
                bitsPerPixel: 32, bytesPerRow: surface.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue),
                provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent) else { throw Refusal.native }
        let candidates = try recognize(image)
        guard candidates.count <= 128 else { throw tagged(.target, "ocr-budget") }
        var bytes = 0, result: [ObservedText] = []
        for (index, candidate) in candidates.enumerated() {
            let box = candidate.box
            guard [box.minX, box.minY, box.width, box.height].allSatisfy({ $0.isFinite }),
                  box.width > 0, box.height > 0, CGRect(x: 0, y: 0, width: 1, height: 1).contains(box),
                  candidate.confidence.isFinite, (0...1).contains(candidate.confidence),
                  candidate.alternativeConfidence.map({ $0.isFinite && (0...1).contains($0) }) ?? true,
                  candidate.text.utf8.count <= 4096 else { throw tagged(.target, "ocr-metadata-unknown") }
            // Scrub before any size reduction so truncation cannot expose a prefix.
            let safe = try scrub(candidate.text)
            bytes += safe.utf8.count
            guard bytes <= 8192 else { throw tagged(.target, "ocr-budget") }
            let overlap = candidates.enumerated().contains { $0.offset != index && $0.element.box.intersects(box) }
            result.append(ObservedText(text: safe, confidence: candidate.confidence,
                bounds: CGRect(x: surface.bounds.minX + box.minX * surface.bounds.width,
                    y: surface.bounds.minY + (1 - box.maxY) * surface.bounds.height,
                    width: box.width * surface.bounds.width, height: box.height * surface.bounds.height),
                ambiguous: overlap || candidate.confidence < 0.8 ||
                    (candidate.alternativeConfidence.map({ candidate.confidence - $0 < 0.15 }) ?? false)))
        }
        try authority()
        return result
    }
}
