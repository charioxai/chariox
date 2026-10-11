import AppKit

struct TargetContext: Equatable {
    let processStart: String
    let stamp: CaptureStamp
    var observer: String = "private"
}
struct TargetMetadata: Equatable {
    let role: String, subrole: String?, label: String?
    let bounds: CGRect
    let enabled: Bool, focused: Bool
    let actions: [String]
}
struct ObservedTarget {
    let ref: String
    let revision: UInt64
    let metadata: TargetMetadata
}
enum NativeObservation {
    case ax([ObservedTarget], CaptureStamp)
    case ocr([ObservedText], CaptureStamp)
}
// Reuse the shared snapshot and OCR result shapes. Bounds are top-left desktop
// pixels; richer private OCR confidence/ambiguity metadata never invents a ref.
func projectNativeObservation(_ observation: NativeObservation, op: String, revision: UInt64) throws -> [String: Any] {
    switch observation {
    case let .ocr(text, stamp):
        if op == "ocr" {
            guard !text.contains(where: { $0.ambiguous }) else { throw tagged(.target, "ocr-ambiguous") }
            return ["text": text.map(\.text).joined(separator: "\n"), "surface_id": "macos-seat", "generation": stamp.generation]
        }
        return ["surface_id": "macos-seat", "generation": stamp.generation, "tree_revision": revision,
                "nodes": [], "complete": false, "fallback": "ocr"]
    case let .ax(targets, stamp):
        var nodes: [[String: Any]] = [], bytes = 0
        for target in targets {
            let metadata = target.metadata, surface = stamp.surface
            let rect = metadata.bounds.intersection(surface.bounds)
            let node: [String: Any] = ["target_id": target.ref, "role": metadata.role, "name": metadata.label ?? "",
                "states": [metadata.enabled ? "enabled" : nil, metadata.focused ? "focused" : nil, "showing"].compactMap { $0 },
                "bounds": [Int((rect.minX - surface.bounds.minX) * surface.scale), Int((rect.minY - surface.bounds.minY) * surface.scale),
                           Int(rect.width * surface.scale), Int(rect.height * surface.scale)],
                "actions": metadata.enabled && metadata.role == kAXButtonRole && metadata.actions.contains(kAXPressAction) ? ["click"] : []]
            let size = try JSONSerialization.data(withJSONObject: node).count + 1
            if bytes + size > 3072 { continue }
            bytes += size; nodes.append(node)
        }
        let complete = nodes.count == targets.count && !nodes.isEmpty
        return ["surface_id": "macos-seat", "generation": stamp.generation, "tree_revision": revision,
                "nodes": nodes, "complete": complete, "fallback": complete ? "none" : "ocr"]
    }
}
// OS reads are injected at the AX boundary. Secure nodes are classified before
// metadata or children, and a failed walk cannot retain an earlier observation.
struct TargetReader<Node> {
    let secure: (Node) throws -> Bool
    let metadata: (Node) throws -> TargetMetadata
    let children: (Node) throws -> [Node]
}
struct ConsumedTarget<Node> {
    let node: Node
    let metadata: TargetMetadata
    let deadline: TimeInterval
    let now: () -> TimeInterval
    let current: () -> Bool
    func perform(validate: (Node, TargetMetadata) throws -> Void,
                 mutation: (Node) throws -> Void) throws {
        try validate(node, metadata)
        // Synchronous AX reads may consume the entire reference lifetime.
        // This is the final check immediately before the caller's mutation.
        guard current(), now() < deadline else { throw tagged(.target, "target-ref-stale") }
        try mutation(node)
    }
}
final class TargetRegistry<Node> {
    private var entries: [String: (node: Node, metadata: TargetMetadata)] = [:]
    private var context: TargetContext?
    private var expires: TimeInterval = 0
    private var revision: UInt64 = 0
    var currentRevision: UInt64 { revision }
    private let now: () -> TimeInterval
    init(now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) { self.now = now }
    func invalidate() { entries.removeAll(); context = nil; revision &+= 1 }
    func observe(root: Node, context: TargetContext, reader: TargetReader<Node>,
                 scrub: (String) throws -> String) throws -> [ObservedTarget] {
        invalidate()
        var complete = false
        defer { if !complete { invalidate() } }
        let deadline = now() + 2
        let observationRevision = revision
        var pending = [(root, 0)], visited = 0, result: [ObservedTarget] = []
        while let (node, depth) = pending.popLast() {
            visited += 1
            guard visited <= 128, depth <= 32, now() < deadline else { throw tagged(.target, "target-coverage-budget") }
            if try reader.secure(node) { continue }
            let metadata = try reader.metadata(node)
            guard metadata.role.utf8.count <= 128, (metadata.subrole?.utf8.count ?? 0) <= 128,
                  metadata.actions.count <= 16, metadata.actions.allSatisfy({ $0.utf8.count <= 128 }),
                  (metadata.label?.utf8.count ?? 0) <= 4096,
                  [metadata.bounds.minX, metadata.bounds.minY, metadata.bounds.width, metadata.bounds.height].allSatisfy({ $0.isFinite }),
                  metadata.bounds.width > 0, metadata.bounds.height > 0 else { throw tagged(.target, "target-metadata-unknown") }
            let label = try metadata.label.map(scrub)
            let safe = TargetMetadata(role: metadata.role, subrole: metadata.subrole,
                label: (label?.utf8.count ?? 0) <= 256 ? label : nil, bounds: metadata.bounds,
                enabled: metadata.enabled, focused: metadata.focused, actions: metadata.actions)
            if !safe.bounds.intersection(context.stamp.surface.bounds).isEmpty {
                let ref = UUID().uuidString
                entries[ref] = (node, safe); result.append(ObservedTarget(ref: ref, revision: revision, metadata: safe))
            }
            let children = try reader.children(node)
            guard children.count <= 128 - visited - pending.count, now() < deadline else { throw tagged(.target, "target-coverage-budget") }
            pending += children.reversed().map { ($0, depth + 1) }
        }
        guard revision == observationRevision else { throw tagged(.target, "target-observation-changed") }
        self.context = context; expires = now() + 2; complete = true
        return result
    }
    func consume(_ ref: String, context: TargetContext,
                 validate: (Node, TargetMetadata) throws -> Bool) throws -> ConsumedTarget<Node> {
        let deadline = expires
        let node = try resolve(ref, context: context, validate: validate)
        guard let metadata = entries[ref]?.metadata else { throw tagged(.target, "target-ref-stale") }
        invalidate()
        let consumedRevision = revision
        return ConsumedTarget(node: node, metadata: metadata, deadline: deadline, now: now,
            current: { self.revision == consumedRevision })
    }
    func resolve(_ ref: String, context: TargetContext, validate: (Node, TargetMetadata) throws -> Bool) throws -> Node {
        do {
            guard self.context == context, now() < expires, let entry = entries[ref],
                  try validate(entry.node, entry.metadata),
                  self.context == context, entries[ref] != nil, now() < expires
            else { throw tagged(.target, "target-ref-stale") }
            return entry.node
        } catch { invalidate(); throw error }
    }
}
