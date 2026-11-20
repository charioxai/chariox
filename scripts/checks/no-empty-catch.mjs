// Fail clearly, never silently: a TypeScript catch block that is empty swallows the error without a trace.
// A listed file may keep today's count; a catch with a comment saying why it is safe to ignore is a decision and passes.
// Comments and string data are not code: `catch {}` in either is not a violation, and a comment inside the braces still marks the catch as decided.
// Review after 2026-12-15: drop the list entries that never shrank or widen the scope if .mjs app code keeps the habit.
import { tsCode } from '../lib/ts-code.mjs'

const SCOPE = /^(apps|packages)\/.*\.tsx?$/
const TEST = /(\.test\.tsx?$|\.spec\.tsx?$|\/tests?\/|\/__tests__\/|\.d\.ts$)/
const EMPTY_CATCH = /\bcatch\s*(?:\([^)]*\))?\s*\{\s*\}/g

export default {
  name: 'no-empty-catch',
  why: 'swallowed an error before this check existed; handle or log it when touching the file',
  rule: 'AGENTS.md "Writing clean code": fail clearly, never silently; every operation ends with a result and a reason',
  rootFix: 'handle the error, log it with the reason, or write the comment that says why ignoring it is safe; then run node scripts/checks/run.mjs --shrink',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (!SCOPE.test(file) || TEST.test(file)) continue
      const text = ctx.read(file)
      if (text === null) continue
      const code = tsCode(text)
      for (const m of code.matchAll(EMPTY_CATCH)) {
        // A comment between the braces is in the raw text only: the catch is decided, not empty.
        if (text.slice(m.index, m.index + m[0].length).replace(/\s+/g, '').length !== m[0].replace(/\s+/g, '').length) continue
        findings.push({ file, line: code.slice(0, m.index).split('\n').length, what: `has an empty catch block: ${m[0].replace(/\s+/g, ' ')}` })
      }
    }
    return findings
  },
}
