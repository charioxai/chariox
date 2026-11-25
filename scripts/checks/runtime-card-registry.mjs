import { productionSource } from '../lib/rust-production-source.mjs'

const root = 'apps/kernel/src/'
const registration = `${root}runtime/state/runtime_interaction_owned_state/`
const session = `${root}session/runtime_session.rs`
const pendingStore = `${root}runtime/state/pending_runtime_state.rs`
const boundaries = [
  [/\bLiveRoomCard\s*::\s*admitted\s*\(/g, new Set([`${registration}card_registry.rs`])],
  [/\.\s*(?:add_active_interaction|insert_interaction)\s*\(/g, new Set([session, `${registration}registration.rs`])],
  [/\bactive_interactions\s*\.\s*push\s*\(/g, new Set([session])],
  [/\bPendingInteraction\s*\{/g, new Set([pendingStore, ...['registration', 'kernel_wide', 'user_domain'].map(file => `${registration}${file}.rs`)])],
]

export default {
  name: 'runtime-card-registry',
  ratchet: 'diff',
  rule: 'Only the kernel card registry may admit or publish a new owner card',
  rootFix: 'use the admitting state machine and the registry in docs/RUNTIME_CARD_RULES.md',
  scan(ctx) {
    const findings = []
    for (const file of ctx.files) {
      if (!file.startsWith(root) || !file.endsWith('.rs')) continue
      const code = productionSource(ctx.read(file) ?? '', file)
      for (const [pattern, allowed] of boundaries) {
        if (allowed.has(file)) continue
        for (const hit of code.matchAll(pattern)) {
          findings.push({ file, line: code.slice(0, hit.index).split('\n').length, what: 'adds a card outside its registry' })
        }
      }
    }
    return findings
  },
}
