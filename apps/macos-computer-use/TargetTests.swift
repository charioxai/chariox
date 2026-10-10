import AppKit

@main struct TargetTests {
    static func main() throws {
        let surface = NativeSurface(bounds: CGRect(x: -100, y: 20, width: 100, height: 100), scale: 2)
        let context = TargetContext(processStart: "p1", stamp: CaptureStamp(generation: "g1", surface: surface, protection: 1))
        var clock: TimeInterval = 0
        let registry = TargetRegistry<String>(now: { clock })
        var secureReads = 0
        var moved = false
        let reader = TargetReader<String>(secure: { $0 == "secure" }, metadata: { node in
            if node == "secure" { secureReads += 1 }
            return TargetMetadata(role: "AXButton", subrole: nil, label: "public canary secret",
                bounds: CGRect(x: moved ? -70 : -90, y: 30, width: 20, height: 20), enabled: true, focused: false, actions: ["AXPress"])
        }, children: { $0 == "root" ? ["secure", "button"] : [] })
        let first = try registry.observe(root: "root", context: context, reader: reader, scrub: { $0.replacingOccurrences(of: "secret", with: "[redacted]") })
        precondition(first.count == 2 && secureReads == 0 && first[1].metadata.label == "public canary [redacted]")
        let resolved = try registry.resolve(first[1].ref, context: context, validate: { _ in !moved })
        precondition(resolved == "button")
        moved = true
        expectRefusal { _ = try registry.resolve(first[1].ref, context: context, validate: { _ in !moved }) }
        moved = false
        expectRefusal { _ = try registry.resolve(first[1].ref, context: context, validate: { _ in true }) }
        let second = try registry.observe(root: "root", context: context, reader: reader, scrub: { $0 })
        expectRefusal { _ = try registry.resolve(first[1].ref, context: context, validate: { _ in true }) }
        let changed = TargetContext(processStart: "reused-pid", stamp: context.stamp)
        expectRefusal { _ = try registry.resolve(second[1].ref, context: changed, validate: { _ in true }) }
        let third = try registry.observe(root: "root", context: context, reader: reader, scrub: { $0 })
        registry.invalidate()
        expectRefusal { _ = try registry.resolve(third[1].ref, context: context, validate: { _ in true }) }
        let fourth = try registry.observe(root: "root", context: context, reader: reader, scrub: { $0 })
        clock = 3
        expectRefusal { _ = try registry.resolve(fourth[1].ref, context: context, validate: { _ in true }) }
        let cycle = TargetReader<String>(secure: { _ in false }, metadata: reader.metadata, children: { [$0] })
        expectRefusal { _ = try registry.observe(root: "root", context: context, reader: cycle, scrub: { $0 }) }
        let duringWalk = TargetReader<String>(secure: reader.secure, metadata: { node in
            registry.invalidate()
            return try reader.metadata(node)
        }, children: reader.children)
        expectRefusal { _ = try registry.observe(root: "root", context: context, reader: duringWalk, scrub: { $0 }) }
        for stamp in [CaptureStamp(generation: "g2", surface: surface, protection: 1),
                      CaptureStamp(generation: "g1", surface: surface, protection: 2),
                      CaptureStamp(generation: "g1", surface: NativeSurface(bounds: surface.bounds, scale: 1), protection: 1)] {
            let snapshot = try registry.observe(root: "root", context: context, reader: reader, scrub: { $0 })
            expectRefusal { _ = try registry.resolve(snapshot[1].ref, context: TargetContext(processStart: "p1", stamp: stamp), validate: { _ in true }) }
        }
        print("PASS secure nodes expose no metadata/children; scrubbed opaque refs expire on movement, failed validation, observation, process reuse, notification and age; cycles refuse")
    }
    static func expectRefusal(_ body: () throws -> Void) {
        do { try body(); preconditionFailure("stale or unbounded target admitted") } catch {}
    }
}
