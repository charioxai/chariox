// Unprivileged AppKit reference for the public TextEdit caret drill. It reads only
// a granted-helper window capture and CG window bounds, never AX, and runs AppKit
// on this process's main thread. The granted helper never builds AppKit views.
import AppKit

let probeText = (1...12).map { String(format: "line %02d public caret probe", $0) }.joined(separator: "\n") + "\n"
let probeIndexes = [("line5-before-r", 125), ("line5-after-r", 126)]
let margin: CGFloat = 10

struct Band { var top: Int, bottom: Int, left: Int, right: Int }
struct Fit: Error { let reason: String }
struct Ink {
    let width: Int, height: Int, bits: [Bool]
    func at(_ x: Int, _ y: Int) -> Bool { x >= 0 && y >= 0 && x < width && y < height && bits[y * width + x] }
}

// Ink differs strongly from the dominant background (light or dark appearance).
func ink(_ image: CGImage) -> Ink {
    var data = [UInt8](repeating: 0, count: image.width * image.height)
    let context = CGContext(data: &data, width: image.width, height: image.height, bitsPerComponent: 8,
                            bytesPerRow: image.width, space: CGColorSpaceCreateDeviceGray(),
                            bitmapInfo: CGImageAlphaInfo.none.rawValue)!
    context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    var histogram = [Int](repeating: 0, count: 256)
    for value in data { histogram[Int(value)] += 1 }
    let background = histogram.indices.max(by: { histogram[$0] < histogram[$1] })!
    return Ink(width: image.width, height: image.height, bits: data.map { abs(Int($0) - background) > 90 })
}

// Rows need several ink pixels, so a 1-2 px caret never forms or bridges a band.
func bands(_ ink: Ink) -> [Band] {
    let inset = 4
    var result: [Band] = [], current: Band?
    for y in inset..<(ink.height - inset) {
        let xs = (inset..<(ink.width - inset)).filter { ink.at($0, y) }
        if xs.count >= 4 {
            if current == nil { current = Band(top: y, bottom: y, left: xs.first!, right: xs.last!) }
            current!.bottom = y
            current!.left = min(current!.left, xs.first!); current!.right = max(current!.right, xs.last!)
        } else if let band = current { result.append(band); current = nil }
    }
    if let band = current { result.append(band) }
    return result
}

// Share of ink pixels (capture and shifted replica) that have ink in the other
// within one pixel, over the text block; tolerant of sub-pixel glyph phase.
func overlap(_ capture: Ink, _ model: Ink, dx: Int, dy: Int, lines: [Band]) -> Double {
    func near(_ ink: Ink, _ x: Int, _ y: Int) -> Bool {
        (-1...1).contains { oy in (-1...1).contains { ox in ink.at(x + ox, y + oy) } }
    }
    var matched = 0, total = 0
    for y in (lines.first!.top - 2)...(lines.last!.bottom + 2) {
        for x in (lines.map(\.left).min()! - 2)...(lines.map(\.right).max()! + 2) {
            if capture.at(x, y) { total += 1; if near(model, x - dx, y - dy) { matched += 1 } }
            if model.at(x - dx, y - dy) { total += 1; if near(capture, x, y) { matched += 1 } }
        }
    }
    return total == 0 ? 0 : Double(matched) / Double(total)
}

func replica(_ font: NSFont) -> NSTextView {
    let view = NSTextView(frame: NSRect(x: 0, y: 0, width: 2000, height: 2000))
    view.textContainerInset = .zero
    view.font = font; view.string = probeText; view.textColor = .black
    view.layoutManager!.ensureLayout(for: view.textContainer!)
    return view
}

// Draws the replica glyphs at `origin` points into a white canvas; `extra` adds
// synthetic window chrome for the self-test.
func render(_ view: NSTextView, size: NSSize, origin: NSPoint, scale: CGFloat,
            extra: (CGFloat) -> Void = { _ in }) -> CGImage {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width * scale), pixelsHigh: Int(size.height * scale),
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    rep.size = size
    NSGraphicsContext.saveGraphicsState()
    let base = NSGraphicsContext(bitmapImageRep: rep)!
    base.cgContext.translateBy(x: 0, y: size.height); base.cgContext.scaleBy(x: 1, y: -1)
    NSGraphicsContext.current = NSGraphicsContext(cgContext: base.cgContext, flipped: true)
    NSColor.white.setFill(); NSRect(origin: .zero, size: size).fill()
    let layout = view.layoutManager!
    layout.drawGlyphs(forGlyphRange: layout.glyphRange(for: view.textContainer!), at: origin)
    extra(size.width)
    NSGraphicsContext.restoreGraphicsState()
    return rep.cgImage!
}

func median(_ values: [Int]) -> Int { values.sorted()[values.count / 2] }

func reference(capture: CGImage, bounds: CGRect) throws -> [String: Any] {
    let scale = CGFloat(capture.width) / bounds.width
    guard [1, 2].contains(scale), CGFloat(capture.height) == bounds.height * scale else { throw Fit(reason: "capture-bounds-mismatch") }
    // Window edges and the title separator span the capture; text lines never do.
    let captured = ink(capture), capturedBands = bands(captured).filter { $0.right - $0.left < captured.width * 9 / 10 }
    guard capturedBands.count >= 12 else { throw Fit(reason: "text-bands-missing") }
    let lines = Array(capturedBands.suffix(12))
    var names = ["Menlo-Regular", "Monaco", "SFMono-Regular", "Courier"]
    if let user = NSFont.userFixedPitchFont(ofSize: 0)?.fontName, !names.contains(user) { names.insert(user, at: 0) }
    var fits: [[String: Any]] = []
    for name in names {
        for size in 9...18 {
            guard let font = NSFont(name: name, size: CGFloat(size)) else { continue }
            let view = replica(font)
            let used = view.layoutManager!.usedRect(for: view.textContainer!)
            let modelInk = ink(render(view, size: NSSize(width: used.width + 2 * margin, height: used.height + 2 * margin),
                                     origin: NSPoint(x: margin, y: margin), scale: scale))
            let model = bands(modelInk)
            guard model.count == 12 else { continue }
            // The caret may raise or widen one line; every other line and all right edges must agree.
            let lefts = zip(lines, model).map { $0.left - $1.left }, rights = zip(lines, model).map { $0.right - $1.right }
            let tops = zip(lines, model).map { $0.top - $1.top }
            let dx = median(lefts), dy = median(tops)
            let caret = lines.indices.filter { abs(tops[$0] - dy) > 1 || abs(lefts[$0] - dx) > 1 }
            let pitch = tops.indices.filter { !caret.contains($0) }.map { abs(tops[$0] - dy) }.max() ?? 0
            let edge = rights.map { abs($0 - dx) }.max()!
            guard caret.count <= 1, edge <= 1 else { continue }
            var probes: [[String: Any]] = []
            for (probe, index) in probeIndexes {
                let layout = view.layoutManager!
                let rect = layout.boundingRect(forGlyphRange: NSRange(location: layout.glyphIndexForCharacter(at: index), length: 1),
                                               in: view.textContainer!)
                let local = CGPoint(x: rect.minX + rect.width * 0.25, y: rect.midY)
                let insertion = view.characterIndexForInsertion(at: local)
                guard insertion == index else { throw Fit(reason: "appkit-reference-mismatch") }
                probes.append(["name": probe, "expectedLocation": index, "referenceLocation": insertion,
                               "point": ["x": bounds.minX + local.x + margin + CGFloat(dx) / scale,
                                         "y": bounds.minY + local.y + margin + CGFloat(dy) / scale]])
            }
            fits.append(["font": name, "fontSize": size, "scale": scale, "offsetPx": ["x": dx, "y": dy],
                         "maxPitchErrorPx": pitch, "maxEdgeErrorPx": edge, "probes": probes,
                         "inkAgreement": overlap(captured, modelInk, dx: dx, dy: dy, lines: lines)])
        }
    }
    // Monospace twins share metrics; glyph shapes decide. Any near-best fit must
    // place every probe identically.
    fits.sort { ($0["inkAgreement"] as! Double) > ($1["inkAgreement"] as! Double) }
    guard let first = fits.first, first["inkAgreement"] as! Double >= 0.85 else {
        throw Fit(reason: "font-unmatched best=" + ((fits.first?["inkAgreement"] as? Double).map { String(format: "%.2f", $0) } ?? "none"))
    }
    func points(_ fit: [String: Any]) -> [CGFloat] {
        (fit["probes"] as! [[String: Any]]).flatMap { probe -> [CGFloat] in
            let point = probe["point"] as! [String: CGFloat]; return [point["x"]!, point["y"]!]
        }
    }
    for fit in fits where fit["inkAgreement"] as! Double > (first["inkAgreement"] as! Double) - 0.05
        && zip(points(fit), points(first)).contains(where: { abs($0 - $1) > 0.5 }) {
        throw Fit(reason: "font-ambiguous")
    }
    var result = first
    result["matchingFonts"] = fits.map { "\($0["font"]!) \($0["fontSize"]!) agreement=\($0["inkAgreement"]!)" }
    return result
}

func selfTest() throws {
    // Synthetic window: title text, traffic lights and a caret beside line 1.
    let view = replica(NSFont(name: "Menlo-Regular", size: 11)!)
    let size = NSSize(width: 600, height: 400), origin = NSPoint(x: 37.5, y: 61)
    for scale: CGFloat in [1, 2] {
        let image = render(view, size: size, origin: origin, scale: scale) { width in
            NSAttributedString(string: "caret-probe-r4.txt", attributes: [.font: NSFont.systemFont(ofSize: 13)])
                .draw(at: NSPoint(x: width / 2 - 50, y: 6))
            for (index, color) in [NSColor.systemRed, .systemYellow, .systemGreen].enumerated() {
                color.setFill(); NSBezierPath(ovalIn: NSRect(x: 12 + CGFloat(index) * 20, y: 8, width: 12, height: 12)).fill()
            }
            // macOS 14+ draws a 2 pt caret reaching above the glyphs.
            NSColor.controlAccentColor.setFill(); NSRect(x: origin.x + 3, y: origin.y - 1, width: 2, height: 15).fill()
            NSColor.gray.setFill(); NSRect(x: 0, y: 392, width: width, height: 3).fill()
        }
        let bounds = CGRect(x: 100, y: 50, width: size.width, height: size.height)
        let result = try reference(capture: image, bounds: bounds)
        let layout = view.layoutManager!
        for probe in result["probes"] as! [[String: Any]] {
            let index = probe["expectedLocation"] as! Int, point = probe["point"] as! [String: CGFloat]
            let rect = layout.boundingRect(forGlyphRange: NSRange(location: index, length: 1), in: view.textContainer!)
            let wanted = CGPoint(x: bounds.minX + origin.x + rect.minX + rect.width * 0.25, y: bounds.minY + origin.y + rect.midY)
            guard probe["referenceLocation"] as? Int == index, abs(point["x"]! - wanted.x) <= 0.5,
                  abs(point["y"]! - wanted.y) <= 0.5 else { throw Fit(reason: "self-test-probe-\(index)-scale-\(scale)") }
        }
        // A capture that does not match the window bounds is refused.
        do { _ = try reference(capture: image, bounds: bounds.insetBy(dx: 10, dy: 0)); throw Fit(reason: "self-test-bounds") }
        catch let error as Fit where error.reason == "capture-bounds-mismatch" {}
    }
    print("PASS TextEdit reference recovers synthetic probes at scale 1 and 2")
}

let arguments = Array(CommandLine.arguments.dropFirst())
do {
    if arguments == ["--self-test"] { try selfTest(); exit(0) }
    let numbers = arguments.dropFirst().compactMap(Double.init)
    guard arguments.count == 5, numbers.count == 4,
          let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: arguments[0]) as CFURL, nil),
          let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw Fit(reason: "usage: CAPTURE.png X Y WIDTH HEIGHT") }
    let result = try reference(capture: image, bounds: CGRect(x: numbers[0], y: numbers[1], width: numbers[2], height: numbers[3]))
    print(String(data: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]), encoding: .utf8)!)
} catch {
    fputs("refused: \((error as? Fit)?.reason ?? "unexpected-error")\n", stderr)
    exit(1)
}
