import { runtimeCardTypesSource } from '../lib/runtime-card-types.mjs'
import { productionSource } from '../lib/rust-production-source.mjs'

const root = 'apps/kernel/src/'
const registration = `${root}runtime/state/runtime_interaction_owned_state/`
const session = `${root}session/runtime_session.rs`
const pendingStore = `${root}runtime/state/pending_runtime_state.rs`
const boundaries = [
  [/\bLiveRoomInteraction\s*::\s*admitted\s*\(/g, new Set([`${registration}card_registry.rs`])],
  [/\.\s*(?:add_active_interaction|insert_interaction)\s*\(/g, new Set([session, `${registration}registration.rs`])],
  [/\bactive_interactions\s*\.\s*push\s*\(/g, new Set([session])],
  [/\bPendingInteraction\s*\{/g, new Set([pendingStore, ...['registration', 'kernel_wide', 'user_domain'].map(file => `${registration}${file}.rs`)])],
]

const typesFile = 'docs/runtime-card-types.json'
const decisionsFile = 'docs/card-decisions.json'
const classes = new Set(['asked-for notification', 'person action or answer', 'live browser card'])

function registrations(ctx) {
  if (!ctx.root && !ctx.files.includes(typesFile)) return []
  const findings = []
  const problem = (file, what) => findings.push({ file, line: 1, what })
  const read = file => {
    try {
      const value = JSON.parse(ctx.read(file) ?? 'null')
      if (!Array.isArray(value)) throw new Error('expected an array')
      return value
    } catch { problem(file, 'missing or invalid tracked card registration data'); return [] }
  }
  const entries = read(typesFile), decisions = read(decisionsFile)
  if (!ctx.files.includes(typesFile) || !ctx.files.includes(decisionsFile)) problem(decisionsFile, "the card list and exact decisions must both be tracked")
  const seen = new Set()
  const oldDecisions = JSON.parse(ctx.readAtBase?.(decisionsFile) ?? '[]')
  for (const decision of decisions) {
    const old = Array.isArray(oldDecisions) && oldDecisions.find(record => record.id === decision.id)
    if (old && JSON.stringify(old) !== JSON.stringify(decision)) problem(decisionsFile, `approval record ${decision.id} changed; retain the source decision and add the new exact type/class decision`)
  }
  for (const entry of entries) {
    if (!entry || typeof entry.type !== 'string' || !entry.type || seen.has(entry.type)) {
      problem(typesFile, 'card types must be individually named and unique'); continue
    }
    seen.add(entry.type)
    if (!classes.has(entry.class)) problem(typesFile, `card ${entry.type} has an unapproved class`)
    // Validate every live registration, so editing the decisions file cannot silently remove its authority.
    const approved = decisions.some(d => d?.approved_by === 'Miguel' && typeof d.id === 'string'
      && typeof d.source === 'string' && d.source.trim() && Array.isArray(d.types)
      && d.types.some(pair => pair.type === entry.type && pair.class === entry.class))
    if (!approved) problem(typesFile, `card ${entry.type} class ${entry.class} needs that type's Miguel decision in ${decisionsFile}`)
  }
  if (ctx.root) {
    const generated = 'packages/kernel-client/src/runtime-card-types.ts'
    if (ctx.read(generated) !== runtimeCardTypesSource(entries)) problem(generated, 'regenerate the client projection from the authoritative card list')
  }
  return findings
}

export default {
  name: 'runtime-card-registry',
  ratchet: 'diff',
  rule: 'Only individually registered card types with their exact Miguel-approved class may be admitted',
  rootFix: 'use the admitting state machine and the registry in docs/RUNTIME_CARD_RULES.md',
  scan(ctx) {
    const findings = registrations(ctx)
    for (const file of ctx.files) {
      if (!file.startsWith(root) || !file.endsWith('.rs')) continue
      const code = productionSource(ctx.read(file) ?? '', file)
      const constructors = file === `${registration}card_registry.rs`
        ? /(?:\.with_card_type|(?:InteractionSurface|Self)\s*::\s*(?:named|Card))\s*\(/g
        : /(?:\.with_card_type|InteractionSurface\s*::\s*(?:named|Card))\s*\(/g
      for (const call of code.matchAll(constructors)) {
        const hit = /^(?:\.with_card_type|(?:InteractionSurface|Self)\s*::\s*(?:named|Card))\s*\(\s*"([^"]+)"\s*\)/.exec((ctx.read(file) ?? '').slice(call.index))
        if (!hit) continue
        const entries = JSON.parse(ctx.read(typesFile) ?? '[]')
        if (!entries.some(entry => entry.type === hit[1])) {
          findings.push({ file, line: code.slice(0, call.index).split('\n').length, what: `unregistered card type ${hit[1]}` })
        }
      }
      const statusSources = new Set(['provider_stall.rs', 'agent_deadline_status.rs', 'agent_task_owner_interactions.rs', 'usage_limit_interactions.rs'].map(name => `${root}runtime/state/${name}`))
      for (const hit of code.matchAll(/InteractionSurface\s*::\s*(?:PaneStatus|Hidden)/g)) {
        if (!statusSources.has(file)) findings.push({ file, line: code.slice(0, hit.index).split('\n').length, what: 'pane status admission belongs to its existing runtime state producer' })
      }
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
