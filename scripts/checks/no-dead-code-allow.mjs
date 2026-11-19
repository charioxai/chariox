// Delete dead code in the same change: a Rust lint allowance that hides unused code keeps it alive.
// A listed file may keep today's count (platform-gated items and items a test reaches); a new allowance anywhere else is red.
// Review after 2026-12-15: drop the list entries that never shrank or narrow the lints if an allowance proves legitimate.
import { productionSource } from '../lib/rust-production-source.mjs'

const SCOPE = /^(apps|packages|adapters)\/.*\.rs$/
// #[allow(dead_code)], #![allow(unused_imports)] and the same inside cfg_attr(..., allow(...)).
const ALLOW = /#!?\[[^\]]*\ballow\(\s*(?:[^)\]]*,\s*)?(dead_code|unused(?:_[a-z_]+)?)\b[^\]]*\]/g

export default {
  name: 'no-dead-code-allow',
  why: 'allowed before this check existed; delete the unused item or make it used when touching the file',
  rule: 'AGENTS.md "Writing clean code": delete dead code, old paths and temporary scaffolding in the same change',
  rootFix: 'delete the unused item, or use it from production code; then run node scripts/checks/run.mjs --shrink',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (!SCOPE.test(file)) continue
      const text = ctx.read(file)
      if (text === null) continue
      const code = productionSource(text, file)
      for (const m of code.matchAll(ALLOW)) findings.push({ file, line: code.slice(0, m.index).split('\n').length, what: `allows ${m[1]} instead of deleting the unused code` })
    }
    return findings
  },
}
