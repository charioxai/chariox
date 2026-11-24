// docs/PERFORMANCE_BUDGETS.md, Kernel: a read request does not build a full Room snapshot. A full snapshot also reads the
// Room's tasks, wakes and queued messages from the durable store and republishes it (~800 ms each under load on b3,
// 2026-11-18, twice per history outline: #1445). A read takes the stored Room with its agents (`session_with_agents`).
import { productionSource } from '../lib/rust-production-source.mjs'

// The files that execute read-only requests: history, outline, transcript and prompt-history reads, and the session
// read controls (list, resolve, state, inspection). Add a file here when a new read request gets its own executor.
export const READ_PATHS = [
  'apps/kernel/src/runtime/history_executor.rs',
  'apps/kernel/src/runtime/session_read_control.rs',
]
// Every builder of whole-Room snapshots: session_snapshot, list_session_snapshots, resolve_session_snapshot,
// list_session_health_snapshots, session_snapshot_without_projection_update.
const FULL_SNAPSHOT = /\.\s*(\w*session\w*_snapshots?\w*)\s*\(/g

export default {
  name: 'no-full-snapshot-on-read-path',
  why: 'present before this check existed',
  rule: 'docs/PERFORMANCE_BUDGETS.md, Kernel: a read request does not build a full Room snapshot',
  rootFix: 'read the stored Room with KernelRuntimeState::session_with_agents (or a narrower store read) instead of a full snapshot',
  scan(ctx) {
    const findings = []
    for (const file of READ_PATHS) {
      const text = ctx.read(file)
      if (text === null) {
        findings.push({ file, line: 1, what: 'read-path file named in no-full-snapshot-on-read-path.mjs no longer exists: update READ_PATHS' })
        continue
      }
      const code = productionSource(text, file)
      for (const m of code.matchAll(FULL_SNAPSHOT)) findings.push({ file, line: code.slice(0, m.index).split('\n').length, what: `builds a full Room snapshot on a read path: ${m[1]}()` })
    }
    return findings
  },
}
