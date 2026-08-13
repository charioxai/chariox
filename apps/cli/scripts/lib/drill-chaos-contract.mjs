import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"

export const DRILL_CHAOS_REPLAY_SCHEMA = "chariox.drill.chaos_replay.v1"
export const DRILL_CHAOS_INVARIANTS_SCHEMA = "chariox.drill.chaos_invariants.v1"
export const DRILL_CHAOS_CONTRACT_SCHEMA = "chariox.drill.chaos_contract.v1"

export const DRILL_CHAOS_FAULT_KINDS = Object.freeze([
  "delay",
  "drop",
  "duplicate",
  "process-death",
  "reorder",
  "route-partition",
  "route-reconnect",
  "stale-callback",
])

export const DRILL_CHAOS_INVARIANT_IDS = Object.freeze([
  "bounded-queues",
  "eventual-client-convergence",
  "monotonic-cursors",
  "no-action-loss",
  "no-duplicate-execution",
  "resource-cleanup",
  "stale-callback-suppression",
  "valid-authority",
])

export function drillChaosContractManifest() {
  return {
    schema: DRILL_CHAOS_CONTRACT_SCHEMA,
    replaySchema: DRILL_CHAOS_REPLAY_SCHEMA,
    invariantsSchema: DRILL_CHAOS_INVARIANTS_SCHEMA,
    faultKinds: [...DRILL_CHAOS_FAULT_KINDS],
    invariantIds: [...DRILL_CHAOS_INVARIANT_IDS],
  }
}

export function validateDrillChaosFaultPlan(faultPlan, source = "chaos fault plan") {
  if (!Array.isArray(faultPlan)) throw new Error(`${source} is not an array`)
  const ids = new Set()
  for (const [index, fault] of faultPlan.entries()) {
    const faultSource = `${source}[${index}]`
    if (!fault || typeof fault !== "object" || Array.isArray(fault)) {
      throw new Error(`${faultSource} is not an object`)
    }
    requireText(fault.id, `${faultSource}.id`)
    if (ids.has(fault.id)) throw new Error(`${source} has duplicate fault id ${fault.id}`)
    ids.add(fault.id)
    if (!DRILL_CHAOS_FAULT_KINDS.includes(fault.kind)) {
      throw new Error(`${faultSource} has unsupported kind ${JSON.stringify(fault.kind)}`)
    }
    if (fault.match !== undefined) validateMatch(fault.match, `${faultSource}.match`)
    if (fault.times !== undefined && (!Number.isSafeInteger(fault.times) || fault.times < 1)) {
      throw new Error(`${faultSource}.times must be a positive integer`)
    }
    for (const field of ["delayMs", "spacingMs", "window"]) {
      if (fault[field] !== undefined && (!Number.isSafeInteger(fault[field]) || fault[field] < 0)) {
        throw new Error(`${faultSource}.${field} must be a non-negative integer`)
      }
    }
    if (fault.copies !== undefined && (!Number.isSafeInteger(fault.copies) || fault.copies < 2)) {
      throw new Error(`${faultSource}.copies must be an integer of at least two`)
    }
    assertNoSecrets(fault, faultSource)
  }
  return faultPlan
}

export function validateDrillChaosInvariantReport(report, source = "chaos invariant report") {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== DRILL_CHAOS_INVARIANTS_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  if (report.status !== "passed" && report.status !== "failed") {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
  if (!Array.isArray(report.checks)) throw new Error(`${source}.checks is not an array`)
  const ids = new Set()
  for (const [index, check] of report.checks.entries()) {
    const checkSource = `${source}.checks[${index}]`
    if (!check || typeof check !== "object" || Array.isArray(check)) {
      throw new Error(`${checkSource} is not an object`)
    }
    if (!DRILL_CHAOS_INVARIANT_IDS.includes(check.id)) {
      throw new Error(`${checkSource} has unknown id ${JSON.stringify(check.id)}`)
    }
    if (ids.has(check.id)) throw new Error(`${source} has duplicate check ${check.id}`)
    ids.add(check.id)
    if (typeof check.ok !== "boolean") throw new Error(`${checkSource}.ok is not boolean`)
    requireText(check.summary, `${checkSource}.summary`)
    if (check.evidence === undefined) throw new Error(`${checkSource}.evidence is missing`)
    assertNoSecrets(check, checkSource)
  }
  if (JSON.stringify([...ids].sort()) !== JSON.stringify(DRILL_CHAOS_INVARIANT_IDS)) {
    throw new Error(`${source} does not cover every chaos invariant`)
  }
  const expectedStatus = report.checks.every((check) => check.ok) ? "passed" : "failed"
  if (report.status !== expectedStatus) throw new Error(`${source}.status does not match checks`)
  return report
}

export function validateDrillChaosReplayBundle(bundle, source = "chaos replay bundle") {
  if (!bundle || typeof bundle !== "object" || Array.isArray(bundle)) {
    throw new Error(`${source} is not an object`)
  }
  if (bundle.schema !== DRILL_CHAOS_REPLAY_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(bundle.schema)}`)
  }
  requireText(bundle.scenario, `${source}.scenario`)
  requireText(bundle.seed, `${source}.seed`)
  if (!Number.isSafeInteger(bundle.seedState) || bundle.seedState < 0) {
    throw new Error(`${source}.seedState is invalid`)
  }
  validateDrillChaosFaultPlan(bundle.faultPlan, `${source}.faultPlan`)
  validateDrillChaosInvariantReport(bundle.invariants, `${source}.invariants`)
  if (!Array.isArray(bundle.trace)) throw new Error(`${source}.trace is not an array`)
  let previousSequence = 0
  let previousTime = 0
  for (const [index, event] of bundle.trace.entries()) {
    const eventSource = `${source}.trace[${index}]`
    if (!event || typeof event !== "object" || Array.isArray(event)) {
      throw new Error(`${eventSource} is not an object`)
    }
    if (!Number.isSafeInteger(event.sequence) || event.sequence !== previousSequence + 1) {
      throw new Error(`${eventSource}.sequence is not contiguous`)
    }
    if (!Number.isSafeInteger(event.atMs) || event.atMs < previousTime) {
      throw new Error(`${eventSource}.atMs is not monotonic`)
    }
    requireText(event.kind, `${eventSource}.kind`)
    previousSequence = event.sequence
    previousTime = event.atMs
    assertNoSecrets(event, eventSource)
  }
  if (!bundle.summary || typeof bundle.summary !== "object" || Array.isArray(bundle.summary)) {
    throw new Error(`${source}.summary is invalid`)
  }
  for (const field of ["traceEvents", "faultsApplied", "staleCallbacksSuppressed"]) {
    if (!Number.isSafeInteger(bundle.summary[field]) || bundle.summary[field] < 0) {
      throw new Error(`${source}.summary.${field} is invalid`)
    }
  }
  if (bundle.summary.traceEvents !== bundle.trace.length) {
    throw new Error(`${source}.summary.traceEvents does not match trace`)
  }
  const faultEvents = bundle.trace.filter((event) => (
    event.kind === "transport.fault-applied" || event.kind === "chaos.fault-applied"
  ))
  if (bundle.summary.faultsApplied !== faultEvents.length) {
    throw new Error(`${source}.summary.faultsApplied does not match trace`)
  }
  const appliedFaultIds = new Set(faultEvents.map((event) => event.details?.faultId))
  const missingFaultIds = bundle.faultPlan
    .map((fault) => fault.id)
    .filter((faultId) => !appliedFaultIds.has(faultId))
  if (missingFaultIds.length > 0) {
    throw new Error(`${source} has faults without trace evidence: ${missingFaultIds.join(", ")}`)
  }
  assertNoSecrets(bundle.metadata ?? {}, `${source}.metadata`)
  return bundle
}

function validateMatch(match, source) {
  if (!match || typeof match !== "object" || Array.isArray(match)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [key, value] of Object.entries(match)) {
    if (!["channel", "from", "messageId", "operationId", "to", "type"].includes(key)) {
      throw new Error(`${source} has unsupported field ${key}`)
    }
    requireText(value, `${source}.${key}`)
  }
}

function requireText(value, source) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${source} must be non-empty text`)
  }
}

function assertNoSecrets(value, source, key = "") {
  if (isSensitiveDrillKey(key)) throw new Error(`${source} contains sensitive field ${key}`)
  if (typeof value === "string") {
    if (looksLikeDrillSecretValue(value)) throw new Error(`${source} contains a secret-looking value`)
    return
  }
  if (!value || typeof value !== "object") return
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertNoSecrets(item, `${source}[${index}]`, key))
    return
  }
  for (const [childKey, childValue] of Object.entries(value)) {
    assertNoSecrets(childValue, `${source}.${childKey}`, childKey)
  }
}
