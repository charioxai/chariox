import { u32Constant, wireShapes } from '../lib/rust-wire-shapes.mjs'

// Protocol changes: a serialized shape a client depends on may not change without the version bump and the snapshot update.
// Diff-aware: it compares the change with its merge base, so there is no allow list to grow.
// A shape change is a field, variant, variant field or serde attribute that appeared or disappeared in a file (scripts/lib/rust-wire-shapes.mjs).
// The version must be higher than at the base: an unchanged or lowered value, or a reformatted declaration, is not a bump.
// Only the local protocol pins its shapes in snapshots; the relay peer protocol has the bump alone.
const LOCAL = { constant: 'LOCAL_DAEMON_PROTOCOL_VERSION', snapshot: true, shapes: /^apps\/kernel\/src\/local\/api\/types(\.rs|\/.*\.rs)$/ }
const RELAY_PEER = { constant: 'RELAY_PEER_PROTOCOL_VERSION', snapshot: false, shapes: /^apps\/kernel\/src\/transport\/relay_peer(\.rs|\/.*\.rs)$/ }
const SNAPSHOT = /^apps\/kernel\/src\/(local\/api\/tests\/protocol_shapes\/|agent_move\/protocol-)/

export default {
  name: 'protocol-shape-needs-version',
  ratchet: 'diff',
  rule: 'AGENTS.md "Protocol changes": when a serialized shape a client depends on changes, bump the protocol version and update the snapshot and hash tests',
  rootFix: 'raise the protocol constant named here, regenerate with pnpm protocol:generate, and update the matching snapshot under protocol_shapes/ in the same change (a shape no snapshot pins needs a new one)',
  scan(ctx) {
    if (!ctx.base) return []
    const changes = ctx.diff()
    const snapshotChanged = [...changes.keys()].some(file => SNAPSHOT.test(file))
    const findings = []
    for (const { constant, snapshot, shapes } of [LOCAL, RELAY_PEER]) {
      // +1 for each shape in a changed file now, -1 for each at the base, netted across all changed files of the protocol:
      // a shape that moved or was reordered, even into another module, nets to nothing.
      const count = new Map()
      for (const [file, rows] of changes) {
        if (!shapes.test(file)) continue
        for (const [text, sign] of [[ctx.read(file), 1], [ctx.readAtBase(file), -1]]) {
          for (const s of text === null ? [] : wireShapes(text, file)) {
            const entry = count.get(s.key) ?? { balance: 0, where: null }
            entry.balance += sign
            // A removed shape is placed on its line in the diff; an added one on its line in the file.
            entry.where ??= { file, line: sign > 0 ? s.line : (rows.find(r => r.text.trim() === s.text)?.line ?? rows[0].line), text: s.text }
            count.set(s.key, entry)
          }
        }
      }
      const shape = [...count.values()].filter(c => c.balance !== 0).map(c => c.where)
      if (!shape.length) continue
      const first = shape[0]
      const { now, before } = versions(ctx, constant, shapes)
      const values = `${constant} is ${now ?? 'not declared'} now, ${before ?? 'not declared'} at the base`
      const what = `changes a serialized shape (${shape.length} field, variant or serde line(s) added or removed, first: ${first.text.slice(0, 60)}; ${values})`
      if (now === null || before === null || now <= before) {
        findings.push({ file: first.file, line: first.line, what: `${what} without raising ${constant}` })
      } else if (snapshot && !snapshotChanged) findings.push({ file: first.file, line: first.line, what: `${what} and raises ${constant}, but no protocol snapshot changed` })
    }
    return findings
  },
}

// The constant's value now and at the merge base, read where it is declared.
function versions(ctx, constant, shapes) {
  const declaring = ctx.files.filter(f => shapes.test(f)).find(f => u32Constant(ctx.read(f) ?? '', constant) !== null)
  if (!declaring) return { now: null, before: null }
  return { now: u32Constant(ctx.read(declaring), constant), before: u32Constant(ctx.readAtBase(declaring) ?? '', constant) }
}
