// docs/PERFORMANCE_BUDGETS.md, Kernel: a periodic loop does not start a subprocess. A `ps` (or any child) per tick costs a
// fork, an exec and a wait on every tick, and on a busy host the child itself is what runs late (#1445: the macOS sampler's
// `ps` timed out under load). Read the kernel's own sources (/proc, a subscription, a cached value) or start the child once.
import { closing, lineOf } from '../lib/rust-scan.mjs'
import { productionSource } from '../lib/rust-production-source.mjs'

const SCOPE = /^apps\/kernel\/src\/.*\.rs$/
// What builds a child process. bounded_process and the receipt waits all take a Command built by one of these.
const SPAWN = /\b(?:process_spawn::command|Command::new)\s*\(/g
// What makes a loop periodic: it waits on a clock between turns.
const PACING = /\b(?:sleep|sleep_until|recv_timeout|interval|interval_at)\s*\(|\.tick\s*\(\s*\)/
const LOOP = /\b(?:loop|while\b[^{;]*)\s*\{/g

// Functions of the file, with the spans of their bodies.
function functions(code) {
  const fns = []
  for (const m of code.matchAll(/\bfn\s+(\w+)\b[^{;]*\{/g)) {
    const open = m.index + m[0].length - 1, end = closing(code, open)
    if (end > 0) fns.push({ name: m[1], open, end })
  }
  return fns
}

// Names of the file's functions that start a child, directly or through another function of the same file.
function spawningFunctions(code, fns) {
  const spawning = new Set()
  for (let changed = true; changed;) {
    changed = false
    for (const fn of fns) {
      if (spawning.has(fn.name)) continue
      const body = code.slice(fn.open, fn.end)
      if (new RegExp(SPAWN.source).test(body) || [...spawning].some(name => new RegExp(`\\b${name}\\b`).test(body))) {
        spawning.add(fn.name)
        changed = true
      }
    }
  }
  return spawning
}

export default {
  name: 'no-spawn-in-periodic-loop',
  why: 'present before this check existed',
  rule: 'docs/PERFORMANCE_BUDGETS.md, Kernel: a periodic loop does not start a subprocess on its ticks',
  rootFix: 'read what the child printed from the kernel itself (/proc, a subscription, a cached value), or start the child once outside the loop',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (!SCOPE.test(file)) continue
      const text = ctx.read(file)
      if (text === null || !/\b(?:loop|while)\b/.test(text)) continue
      const code = productionSource(text, file)
      const spawning = spawningFunctions(code, functions(code))
      const seen = new Set()
      for (const m of code.matchAll(LOOP)) {
        const open = m.index + m[0].length - 1, end = closing(code, open)
        if (end < 0) continue
        const body = code.slice(open, end)
        if (!PACING.test(body)) continue
        const hits = [...body.matchAll(SPAWN)].map(s => [s.index, 'starts a subprocess'])
        for (const name of spawning) {
          for (const s of body.matchAll(new RegExp(`\\b${name}\\b`, 'g'))) hits.push([s.index, `calls ${name}, which starts a subprocess`])
        }
        for (const [at, what] of hits) {
          const line = lineOf(code, open + at)
          if (seen.has(line)) continue
          seen.add(line)
          findings.push({ file, line, what: `periodic loop ${what}` })
        }
      }
    }
    return findings
  },
}
