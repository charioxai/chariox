import AppKit

private final class FailingEncoder: ProtectedVideoEncoding {
    var calls = 0
    func encode(_ frame: ProtectedFrame, sequence: UInt64) throws -> Data {
        calls += 1
        if calls > 1 { throw tagged(.native, "test-encoder-unavailable") }
        return Data([0,0,0,1,0x65])
    }
}

@main struct CodecTests {
    @MainActor static func main() async throws {
        var clock: TimeInterval = 1
        let attachment = CaptureAttachment(now: { clock })
        let surface = NativeSurface(bounds: CGRect(x: -10, y: 0, width: 128, height: 128), scale: 1)
        let stamp = CaptureStamp(generation: "g", surface: surface, protection: 1)
        var pixels = [UInt8](repeating: 0, count: 128 * 128 * 4)
        for i in stride(from: 3, to: pixels.count, by: 4) { pixels[i] = 255 }
        let probe = MacVideoEncoder(width: 128, height: 128, bitrate: 2_000_000)!
        print("Actual selected backend", probe.backend)
        let id = try attachment.subscribe(owner: "v", generation: "g", codecs: ["avc1.420033", "png"], bitrate: 2_000_000)
        func sample(_ after: UInt64?, repair: Bool = false) async throws -> [String: Any]? {
            try await attachment.sample(id: id, owner: "v", after: after, repair: repair, authority: {},
                capture: { ProtectedFrame(pixels: pixels, stamp: stamp, masked: true) })
        }
        let first = try await sample(nil); precondition(first?["kind"] as? String == "png")
        pixels[0] = 255
        let motion = try await sample(1)!
        // This machine must prove whether VT is usable; fallback remains tested separately.
        precondition(motion["kind"] as? String == "video", "VideoToolbox unavailable, no video component PASS")
        precondition(motion["codec"] as? String == "avc1.420033" && motion["key"] as? Bool == true)
        let bytes = Data(base64Encoded: motion["data_base64"] as! String)!
        precondition(Array(bytes.prefix(4)) == [0, 0, 0, 1] && bytes[4] & 31 == 7)
        precondition(bytes[5] == 0x42 && bytes[6] == 0 && bytes[7] == 0x33)
        try bytes.write(to: URL(fileURLWithPath: "/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r11-20261011/public-protected.h264"))
        let quiet = try await sample(2); precondition(quiet == nil)
        clock += 3
        let settled = try await sample(2); precondition(settled?["kind"] as? String == "png", "settled video must repair exact pixels")
        pixels[4] = 255
        let repaired = try await sample(3, repair: true); precondition(repaired?["kind"] as? String == "png")
        let software = CaptureAttachment(encoderFactory: { _, _, _ in nil })
        let fallback = try software.subscribe(owner: "v", generation: "g", codecs: ["avc1.420033", "png"])
        _ = try await software.sample(id: fallback, owner: "v", after: nil, repair: false, authority: {}, capture: { ProtectedFrame(pixels: pixels, stamp: stamp, masked: true) })
        pixels[8] = 255
        let exact = try await software.sample(id: fallback, owner: "v", after: 1, repair: false, authority: {}, capture: { ProtectedFrame(pixels: pixels, stamp: stamp, masked: true) })!
        precondition(exact["kind"] as? String == "tiles")
        let failingEncoder = FailingEncoder()
        let switching = CaptureAttachment(encoderFactory: { _, _, _ in failingEncoder })
        let switchingID = try switching.subscribe(owner: "v", generation: "g", codecs: ["avc1.420033", "png"])
        func switched(_ after: UInt64?) async throws -> [String: Any] {
            try await switching.sample(id: switchingID, owner: "v", after: after, repair: false, authority: {},
                capture: { ProtectedFrame(pixels: pixels, stamp: stamp, masked: true) })!
        }
        _ = try await switched(nil)
        pixels[12] = 255
        let video = try await switched(1); precondition(video["kind"] as? String == "video")
        pixels[16] = 255
        let failedVideo = try await switched(2)
        precondition(failedVideo["kind"] as? String == "png", "tiles cannot repair a lossy full-frame base")
        let decoded = NSBitmapImageRep(data: Data(base64Encoded: failedVideo["data_base64"] as! String)!)!
        precondition(decoded.pixelsWide == 128 && decoded.pixelsHigh == 128)
        do { _ = try attachment.subscribe(owner: "v", generation: "g", codecs: ["avc1.420033"]); preconditionFailure("no exact fallback") } catch {}
        do { _ = try annexB(Data([0,0,0,8,1]), parameterSets: []); preconditionFailure("truncated NAL") } catch {}
        print("PASS negotiated protected H264, SPS/PPS, quiet source, settled exact repair and PNG fallback")
    }
}
