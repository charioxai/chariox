import AppKit

// Evidence replay only. It reads protected public-fixture PNGs and exercises the
// actual private attachment. It never starts a helper or calls permission APIs.
@main struct CaptureReplay {
    @MainActor static func main() async throws {
        let args = CommandLine.arguments
        guard args.count == 4, let scale = Double(args[3]), [1, 2].contains(scale),
              let first = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: args[1])))?.cgImage,
              let second = NSBitmapImageRep(data: try Data(contentsOf: URL(fileURLWithPath: args[2])))?.cgImage,
              first.width == second.width, first.height == second.height else { throw Refusal.arguments }
        let surface = NativeSurface(bounds: CGRect(x: 0, y: 0, width: Double(first.width) / scale,
                                                   height: Double(first.height) / scale), scale: scale)
        let stamp = CaptureStamp(generation: "evidence-replay", surface: surface, protection: 1)
        let a = try protectedRaster(first, surface: surface, masks: [], wholeWindowMask: false).pixels
        let b = try protectedRaster(second, surface: surface, masks: [], wholeWindowMask: false).pixels
        guard a != b else { throw tagged(.native, "live-capture-no-damage") }
        let attachment = CaptureAttachment()
        let id = try attachment.subscribe(owner: "evidence-replay", generation: stamp.generation)
        func frame(_ pixels: [UInt8]) -> ProtectedFrame { ProtectedFrame(pixels: pixels, stamp: stamp, masked: false) }
        let base = try await attachment.sample(id: id, owner: "evidence-replay", after: nil, repair: false,
                                               authority: {}, capture: { frame(a) })!
        let damage = try await attachment.sample(id: id, owner: "evidence-replay", after: 1, repair: false,
                                                 authority: {}, capture: { frame(b) })!
        let exact = try await attachment.sample(id: id, owner: "evidence-replay", after: 0, repair: true,
                                                authority: {}, capture: { frame(b) })!
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: [base, damage, exact]))
    }
}
