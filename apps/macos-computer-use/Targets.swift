import AppKit

struct TargetContext: Equatable {
    let processStart: String
    let stamp: CaptureStamp
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
// OS reads are injected at the AX boundary. Secure nodes are classified before
// metadata or children, and a failed walk cannot retain an earlier observation.
struct TargetReader<Node> {
    let secure: (Node) throws -> Bool
    let metadata: (Node) throws -> TargetMetadata
    let children: (Node) throws -> [Node]
}
final class TargetRegistry<Node> {
    private var entries: [String: Node] = [:]
    private var context: TargetContext?
    private var expires: TimeInterval = 0
    private var revision: UInt64 = 0
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
                entries[ref] = node; result.append(ObservedTarget(ref: ref, revision: revision, metadata: safe))
            }
            let children = try reader.children(node)
            guard children.count <= 128 - visited - pending.count, now() < deadline else { throw tagged(.target, "target-coverage-budget") }
            pending += children.reversed().map { ($0, depth + 1) }
        }
        guard revision == observationRevision else { throw tagged(.target, "target-observation-changed") }
        self.context = context; expires = now() + 2; complete = true
        return result
    }
    func resolve(_ ref: String, context: TargetContext, validate: (Node) throws -> Bool) throws -> Node {
        do {
            guard self.context == context, now() < expires, let node = entries[ref],
                  try validate(node) else { throw tagged(.target, "target-ref-stale") }
            return node
        } catch { invalidate(); throw error }
    }
}
