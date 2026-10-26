// MP-08 / MP-10 / MP-11 H10/H12: preparation gates, no solver or scorer authority.
import { createHash } from 'node:crypto'
import { sanitizeDrillMetadata } from '../../lib/drill-secrets.mjs'
import { roomProviderToolName } from '../../lib/room-provider-tool-record.mjs'

export const sha256 = text => createHash('sha256').update(text).digest('hex')
const positive = v => Number.isSafeInteger(v) && v > 0
const digest = v => typeof v === 'string' && /^[a-f0-9]{64}$/.test(v)

export function admitSites({ shops, generation, sourceDigest, resetAtMs, nowMs, maxAgeMs = 30000 }) {
  if (!generation || !digest(sourceDigest) || !positive(maxAgeMs) || !Number.isFinite(nowMs)
    || !Number.isFinite(resetAtMs) || resetAtMs > nowMs || shops?.length !== 4
    || new Set(shops.map(s => s.id)).size !== 4) throw new Error('MP-10 invalid four-shop admission identity')
  const fields = ['id', 'generation', 'sourceDigest', 'serviceHealthy', 'jvmCompatible', 'searchHealthy', 'indexedProducts', 'expectedProducts', 'indexedAtMs', 'observedAtMs']
  for (const shop of shops) {
    if (Object.keys(shop).some(key => !fields.includes(key)) || !shop.id || shop.generation !== generation || shop.sourceDigest !== sourceDigest
      || shop.serviceHealthy !== true || shop.jvmCompatible !== true || shop.searchHealthy !== true
      || !positive(shop.indexedProducts) || !positive(shop.expectedProducts) || shop.indexedProducts !== shop.expectedProducts
      || !Number.isFinite(shop.indexedAtMs) || shop.indexedAtMs < resetAtMs
      || !Number.isFinite(shop.observedAtMs) || shop.observedAtMs < shop.indexedAtMs
      || shop.observedAtMs > nowMs || nowMs - shop.observedAtMs > maxAgeMs) {
      throw new Error('MP-10 shop readiness/generation/index RED')
    }
  }
  return { mpItems: ['MP-08', 'MP-10', 'MP-11'], generation, sourceDigest, resetAtMs,
    admittedAtMs: nowMs, expiresAtMs: Math.min(...shops.map(s => s.observedAtMs + maxAgeMs)),
    shops: structuredClone(shops), status: 'admitted', scoredCampaign: false }
}

// Recheck immediately before prompt submission: a prior HTML 200 is not readiness.
export function checkSiteAdmission(receipt, { nowMs, generation, sourceDigest }) {
  if (nowMs > receipt.expiresAtMs || nowMs < receipt.admittedAtMs || generation !== receipt.generation
    || sourceDigest !== receipt.sourceDigest) throw new Error('MP-10 stale site admission')
  return admitSites({ ...receipt, nowMs, maxAgeMs: receipt.expiresAtMs - Math.min(...receipt.shops.map(s => s.observedAtMs)) })
}

export class AttemptLedger {
  #calls = new Map()
  #violations = []
  #mutations = 0
  #settled = false
  constructor({ attemptId, taskId, promptId, maxCalls, maxMutations, tools }) {
    if (![attemptId, taskId, promptId].every(v => typeof v === 'string' && v.length) || !positive(maxCalls)
      || !Number.isSafeInteger(maxMutations) || maxMutations < 0 || !tools || !Object.keys(tools).length
      || Object.entries(tools).some(([name, mode]) => roomProviderToolName(name) !== name || !['read', 'mutation'].includes(mode))) {
      throw new Error('MP-10 invalid declared attempt policy')
    }
    this.identity = { attemptId, taskId, promptId, maxCalls, maxMutations, tools: Object.freeze({ ...tools }) }
    Object.freeze(this.identity)
  }
  admit({ callId, tool, mutationUnits }) {
    if (this.#settled || typeof callId !== 'string' || !callId) throw new Error('MP-10 invalid call identity/settlement')
    const canonical = roomProviderToolName(tool)
    const previous = this.#calls.get(callId)
    if (previous) {
      if (previous.rawTool !== tool || previous.mutationUnits !== mutationUnits) {
        this.#violations.push('conflicting duplicate call'); throw new Error('MP-10 conflicting duplicate call')
      }
      return { ...previous, duplicate: true } // receipt only; must never dispatch again
    }
    const mode = Object.hasOwn(this.identity.tools, canonical) ? this.identity.tools[canonical] : null
    let cause = null
    if (!mode) cause = 'forbidden_tool'
    else if (!Number.isSafeInteger(mutationUnits) || mutationUnits < 0 || (mode === 'read' ? mutationUnits !== 0 : mutationUnits < 1)) cause = 'invalid_mutation_accounting'
    const row = { callId, rawTool: tool, tool: canonical, mutationUnits, admitted: false }
    this.#calls.set(callId, row)
    if (!cause) this.#mutations += mutationUnits
    if (!cause && this.#calls.size > this.identity.maxCalls) cause = 'call_cap'
    if (!cause && this.#mutations > this.identity.maxMutations) cause = 'mutation_cap'
    if (cause) { this.#violations.push(cause); row.cause = cause; return { ...row } }
    row.admitted = true
    return { ...row }
  }
  settle({ lifecycle, finalText }) {
    if (this.#settled) throw new Error('MP-10 attempt already settled; no retry')
    this.#settled = true
    const admittedText = sanitizeDrillMetadata(finalText)
    if (admittedText !== finalText) this.#violations.push('secret_like_final')
    return { mpItems: ['MP-08', 'MP-10', 'MP-11'], ...this.identity, lifecycle,
      finalText: admittedText, finalSha256: typeof finalText === 'string' && admittedText === finalText ? sha256(finalText) : null,
      calls: sanitizeDrillMetadata([...this.#calls.values()].map(v => ({ ...v }))), mutationUnits: this.#mutations,
      violations: [...this.#violations], denominatorIncluded: true,
      valid: lifecycle === 'completed' && typeof finalText === 'string' && finalText.length > 0 && !this.#violations.length }
  }
}
