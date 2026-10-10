import AppKit

@main struct AttachmentTests {
    @MainActor static func main() async throws {
        let source = CaptureAttachment()
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 20, width: 65, height: 40), scale: 2)
        let stamp = CaptureStamp(generation: "g1", surface: surface, protection: 1)
        var pixels = [UInt8](repeating: 0, count: 130 * 80 * 4)
        for offset in stride(from: 3, to: pixels.count, by: 4) { pixels[offset] = 255 }
        let id = try source.subscribe(owner: "viewer-a", generation: "g1")
        func frame() -> ProtectedFrame { ProtectedFrame(pixels: pixels, stamp: stamp, masked: false) }
        let first = try await source.sample(id: id, owner: "viewer-a", after: nil, repair: false,
                                            authority: {}, capture: { frame() })!
        precondition(first["kind"] as? String == "png" && first["sequence"] as? UInt64 == 1)
        precondition(first["surface_id"] as? String == "macos-seat" && first["colour"] as? String == "srgb")
        let quiet = try await source.sample(id: id, owner: "viewer-a", after: 1, repair: false,
                                            authority: {}, capture: { frame() })
        precondition(quiet == nil)
        pixels[(79 * 130 + 129) * 4] = 255
        let patch = try await source.sample(id: id, owner: "viewer-a", after: 1, repair: false,
                                            authority: {}, capture: { frame() })!
        precondition(patch["kind"] as? String == "tiles" && patch["base_sequence"] as? UInt64 == 1)
        let tiles = patch["tiles"] as! [[String: Any]]
        precondition(tiles.count == 1 && tiles[0]["x"] as? Int == 128 && tiles[0]["height"] as? Int == 16)
        let tile = NSBitmapImageRep(data: Data(base64Encoded: tiles[0]["data_base64"] as! String)!)!
        precondition(tile.pixelsWide == 2 && tile.pixelsHigh == 16)
        var bytes = [UInt](repeating: 0, count: 4)
        tile.getPixel(&bytes, atX: 1, y: 15)
        precondition(bytes == [255, 0, 0, 255])
        let repair = try await source.sample(id: id, owner: "viewer-a", after: 0, repair: false,
                                             authority: {}, capture: { frame() })!
        precondition(repair["kind"] as? String == "png" && repair["base_sequence"] == nil)
        source.invalidate()
        let protected = try await source.sample(id: id, owner: "viewer-a", after: 3, repair: false,
                                                authority: {}, capture: { frame() })!
        precondition(protected["kind"] as? String == "png")
        var foreignCaptures = 0
        do {
            _ = try await source.sample(id: id, owner: "viewer-b", after: nil, repair: false,
                authority: {}, capture: { foreignCaptures += 1; return frame() })
            preconditionFailure("foreign viewer admitted")
        } catch { precondition(foreignCaptures == 0) }
        source.close(id: id, owner: "viewer-b")
        _ = try await source.sample(id: id, owner: "viewer-a", after: 4, repair: true,
                                   authority: {}, capture: { frame() })
        source.close(id: id, owner: "viewer-a")
        do {
            _ = try await source.sample(id: id, owner: "viewer-a", after: nil, repair: false,
                                        authority: {}, capture: { frame() })
            preconditionFailure("closed subscription admitted")
        } catch {}
        let revokedID = try source.subscribe(owner: "viewer-a", generation: "g1")
        var checks = 0
        var rejected = false
        do {
            _ = try await source.sample(id: revokedID, owner: "viewer-a", after: nil, repair: false,
                authority: { checks += 1; if checks == 3 { source.invalidate() } }, capture: { frame() })
        } catch { rejected = true }
        precondition(rejected, "protection changed at final commit must withhold encoded pixels")
        var attempted = 0
        var nestedRejected = false
        _ = try await source.sample(id: revokedID, owner: "viewer-a", after: nil, repair: true,
            authority: {}, capture: {
                do {
                    _ = try await source.sample(id: revokedID, owner: "viewer-a", after: nil, repair: true,
                        authority: {}, capture: { attempted += 1; return frame() })
                } catch { nestedRejected = true }
                return frame()
            })
        precondition(nestedRejected && attempted == 0)
        source.close()
        var clock: TimeInterval = 10
        let expiring = CaptureAttachment(now: { clock })
        let expiredID = try expiring.subscribe(owner: "viewer-a", generation: "g1")
        clock = 71
        do {
            _ = try await expiring.sample(id: expiredID, owner: "viewer-a", after: nil, repair: true,
                authority: {}, capture: { attempted += 1; return frame() })
            preconditionFailure("expired viewer admitted")
        } catch { precondition(attempted == 0) }
        print("PASS final protection commit fence, overlapping credit refusal and slow-viewer expiry")
        print("PASS caller-bound native subscription, exact PNG/tile decode, quiet credit, lost-base repair, protection flush and close")
    }
}
