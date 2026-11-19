// Rooms always live under a Project: a Room is built with a non-empty ProjectId, which is required in durable storage and the protocol.
// Review after 2026-12-15: drop a pattern that never fired, or widen the scope if sessions grow a second wire type.
import { productionSource } from '../lib/rust-production-source.mjs'

const KERNEL = /^apps\/kernel\/src\/.*\.rs$/
const SESSION_TYPES = /^apps\/kernel\/src\/(session\/|local\/api\/types\/session)/
// Anywhere in the kernel, a Room's Project is not optional and not built empty or assigned later.
const EVERYWHERE = [
  [/\bOption<\s*(?:crate::session::)?ProjectId\s*>/g, 'makes the ProjectId optional'],
  [/\bproject_id\s*:\s*String::(?:new|default)\(\)/g, 'builds an empty project id'],
  [/\bassign_project_id\b/g, 'assigns the project id after the Room was built'],
]
// In the session and its wire types, the field itself is required.
const SESSION_ONLY = [
  [/\bproject_id\s*:\s*Option<[^;,}]*/g, 'makes the session project_id optional'],
  [/#\[serde\([^\]]*\b(?:default|skip_serializing_if)\b[^\]]*\)\]\s*(?:pub(?:\([^)]*\))?\s+)?project_id\s*:/g, 'lets a stored or sent session omit its project_id'],
]

export default {
  name: 'project-id-required',
  why: 'present before this check existed',
  rule: 'Product rule (Miguel 2026-11-17): Rooms always live under a Project, so a Room is built with a non-empty ProjectId and the id is required in storage and in the protocol',
  rootFix: 'build the Room with a ProjectId from ProjectId::new (checked once where the id enters the kernel) and keep project_id required, not Option, not defaulted',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (!KERNEL.test(file)) continue
      const text = ctx.read(file)
      if (text === null) continue
      const code = productionSource(text, file)
      for (const [pattern, what] of [...EVERYWHERE, ...(SESSION_TYPES.test(file) ? SESSION_ONLY : [])]) {
        for (const m of code.matchAll(pattern)) findings.push({ file, line: code.slice(0, m.index).split('\n').length, what: `${what}: ${m[0].replace(/\s+/g, ' ').slice(0, 60)}` })
      }
    }
    return findings
  },
}
