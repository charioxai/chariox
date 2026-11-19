// docs/PERFORMANCE_BUDGETS.md, Kernel: a client does not re-read on a timer what the kernel pushes. A Room's state, its
// agents, its history outline and the Room list reach every attached client as kernel events (SessionSnapshot,
// AgentTreeChanged, AgentTurnChanged / AssistantMessageCompleted, SessionMetadataChanged / SessionUnavailable); a timer
// that reads them again costs the kernel a full read per client per tick whether anything changed or not.
import { closing, lineOf } from '../lib/rust-scan.mjs'
import { tsCode } from '../lib/ts-code.mjs'

const SCOPE = /^(?:apps|packages)\/[^/]+\/src\/.*\.(?:ts|tsx)$/
const TEST = /(?:\.test\.|\.spec\.|\/__tests__\/|\/test\/|\/tests\/)/
// The reads whose changes the kernel pushes, as request kinds and as kernel-client method names.
export const PUSHED_READS = ['GetSessionState', 'ListSessions', 'ListAgents', 'GetSessionHistoryOutline']
const READ = new RegExp(`\\b(${PUSHED_READS.flatMap(kind => [kind, kind[0].toLowerCase() + kind.slice(1)]).join('|')})\\b`, 'g')
// A timer callback, or a loop that waits on a clock between turns.
const TIMER = /\bsetInterval\s*\(/g
const LOOP = /\b(?:while\s*\([^)]*\)|for\s*\(\s*;\s*;\s*\))\s*\{/g
const PACING = /\bawait\s+(?:delay|sleep|setTimeout|new\s+Promise\s*\(\s*(?:\(\s*)?\w*\s*\)?\s*=>\s*setTimeout)\b/
const FUNCTION = /\b(?:function\s*\*?\s*(\w+)\s*\([^)]*\)[^{]*|(?:const|let)\s+(\w+)\s*=\s*(?:async\s*)?(?:\([^)]*\)|\w+)\s*(?::[^=]*)?=>\s*)\{/g

// A call of the same-file function, not of a property that shares its name (other.refresh()).
const calls = (name, flags) => new RegExp(`(?<![.\\w$])${name}\\s*\\(`, flags)
// A timer handed a function by name runs it each tick: setInterval(refresh, 1000).
const NAMED_CALLBACK = /^\(\s*(\w+)\s*[,)]/

// The file's functions whose bodies read a pushed read, directly or through another function of the same file.
function readingFunctions(code) {
  const fns = []
  for (const m of code.matchAll(FUNCTION)) {
    const open = m.index + m[0].length - 1, end = closing(code, open)
    if (end > 0) fns.push({ name: m[1] ?? m[2], body: code.slice(open, end) })
  }
  const reading = new Map()
  for (let changed = true; changed;) {
    changed = false
    for (const fn of fns) {
      if (reading.has(fn.name)) continue
      const direct = new RegExp(READ.source).exec(fn.body)?.[1]
      const through = [...reading.keys()].find(name => calls(name).test(fn.body))
      if (direct || through) { reading.set(fn.name, direct ?? reading.get(through)); changed = true }
    }
  }
  return reading
}

export default {
  name: 'no-polling-where-subscription',
  why: 'present before this check existed',
  rule: 'docs/PERFORMANCE_BUDGETS.md, Kernel: no fixed-interval polling of a read the kernel pushes as an event',
  rootFix: 'react to the kernel event that carries the change (SessionSnapshot, AgentTreeChanged, AgentTurnChanged, SessionMetadataChanged) and read once on it',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (!SCOPE.test(file) || TEST.test(file)) continue
      const text = ctx.read(file)
      if (text === null || !READ.test(text)) continue
      READ.lastIndex = 0
      const code = tsCode(text)
      const spans = []
      for (const m of code.matchAll(TIMER)) {
        const open = m.index + m[0].length - 1, end = closing(code, open)
        if (end > 0) spans.push([open, end, 'setInterval'])
      }
      for (const m of code.matchAll(LOOP)) {
        const open = m.index + m[0].length - 1, end = closing(code, open)
        if (end > 0 && PACING.test(code.slice(open, end))) spans.push([open, end, 'a loop that sleeps'])
      }
      const reading = readingFunctions(code)
      const seen = new Set()
      for (const [open, end, how] of spans) {
        const body = code.slice(open, end)
        const hits = [...body.matchAll(READ)].map(r => [r.index, r[1]])
        for (const [name, read] of reading) {
          for (const c of body.matchAll(calls(name, 'g'))) hits.push([c.index, `${read} (through ${name})`])
        }
        const named = how === 'setInterval' && NAMED_CALLBACK.exec(body)?.[1]
        if (reading.has(named)) hits.push([body.indexOf(named), `${reading.get(named)} (through ${named})`])
        for (const [at, read] of hits) {
          const line = lineOf(code, open + at)
          if (seen.has(line)) continue
          seen.add(line)
          findings.push({ file, line, what: `${how} re-reads ${read}, which the kernel pushes as an event` })
        }
      }
    }
    return findings
  },
}
