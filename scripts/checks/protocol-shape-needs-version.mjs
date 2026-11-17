// Protocol changes: a serialized shape a client depends on may not change without the version bump and the snapshot update.
// Diff-aware: it compares the change with its merge base, so there is no allow list to grow.
// Only the local protocol pins its shapes in snapshots; the relay peer protocol has the bump alone.
const LOCAL = { constant: 'LOCAL_DAEMON_PROTOCOL_VERSION', snapshot: true, shapes: /^apps\/kernel\/src\/local\/api\/types(\.rs|\/.*\.rs)$/ }
const RELAY_PEER = { constant: 'RELAY_PEER_PROTOCOL_VERSION', snapshot: false, shapes: /^apps\/kernel\/src\/transport\/relay_peer(\.rs|\/.*\.rs)$/ }
const SNAPSHOT = /^apps\/kernel\/src\/(local\/api\/tests\/protocol_shapes\/|agent_move\/protocol-)/
// A field, an enum variant or a serde attribute: the lines that decide what goes on the wire.
const SHAPE_LINE = /^\s*(pub\s+[a-z_]\w*\s*:|[A-Z]\w*\s*(,|\(|\{|=)|#\[serde\()/
const COMMENT = /^\s*(\/\/|\*|\/\*)/

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
      const bumped = [...changes.values()].some(rows => rows.some(r => r.sign === '+' && new RegExp(`\\b${constant}\\s*:\\s*u32\\s*=`).test(r.text)))
      // A moved or reordered field nets to nothing; only lines that really appeared or disappeared change the wire.
      const net = new Map()
      for (const [file, rows] of changes) {
        if (!shapes.test(file)) continue
        for (const r of rows.filter(r => SHAPE_LINE.test(r.text) && !COMMENT.test(r.text))) {
          const key = r.text.trim()
          const entry = net.get(key) ?? { balance: 0, file, line: r.line }
          entry.balance += r.sign === '+' ? 1 : -1
          net.set(key, entry)
        }
      }
      const shape = [...net].filter(([, e]) => e.balance !== 0)
      if (!shape.length) continue
      const [text, first] = shape[0]
      const what = `changes a serialized shape (${shape.length} field, variant or serde line(s) added or removed, first: ${text.slice(0, 60)})`
      if (!bumped) findings.push({ file: first.file, line: first.line, what: `${what} without raising ${constant}` })
      else if (snapshot && !snapshotChanged) findings.push({ file: first.file, line: first.line, what: `${what} and raises ${constant}, but no protocol snapshot changed` })
    }
    return findings
  },
}
