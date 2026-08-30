import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"
import { validateDrillTimestampOrder } from "./drill-time.mjs"

export const BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA = "chariox.browser_computer.functional_evidence.v1"

const CASES = deepFreeze([
  functionalCase("browser.discovery", "browser", [
    "url-title-and-lifecycle",
    "accessible-fields-buttons-and-links",
    "stable-tab-identity",
  ]),
  functionalCase("browser.form", "browser", [
    "find-fill-click-and-submit",
    "auto-wait-before-action",
    "mutation-completes-exactly-once",
  ]),
  functionalCase("browser.persistence", "browser", [
    "cookie-restored",
    "local-storage-restored",
    "indexed-db-restored",
    "cache-storage-restored",
    "service-worker-restored",
  ]),
  functionalCase("browser.structures", "browser", [
    "nested-frame-target",
    "shadow-root-target",
    "stale-reference-rejected",
  ]),
  functionalCase("browser.lifecycle", "browser", [
    "dialog-handled",
    "popup-registered",
    "download-completed",
    "upload-completed",
    "missing-target-times-out",
  ]),
  functionalCase("computer.observe", "computer", [
    "screenshot-matches-active-display",
    "ocr-finds-visible-text",
    "find-text-returns-display-coordinates",
  ]),
  functionalCase("computer.pointer", "computer", [
    "move-click-double-click-and-right-click",
    "drag-and-scroll",
    "input-completes-or-cancels-on-stream-loss",
  ]),
  functionalCase("computer.text-selection", "computer", [
    "drag-selects-text",
    "selection-does-not-move-containing-window",
  ]),
  functionalCase("computer.keyboard", "computer", [
    "text-shortcut-and-key-chord",
    "non-us-text-sample",
    "input-target-remains-focused",
  ]),
  functionalCase("computer.clipboard", "computer", [
    "human-agent-and-slice-directions",
    "secret-copy-restricted",
  ]),
  functionalCase("computer.desktop", "computer", [
    "browser-chrome-controllable",
    "non-browser-program-controllable",
    "canonical-resize-reaches-all-viewers",
  ]),
  functionalCase("shared.single-environment", "shared", [
    "one-room-environment",
    "one-browser-and-profile",
    "web-and-tui-observe-same-state",
  ]),
  functionalCase("shared.takeover", "shared", [
    "human-pauses-or-cancels-agent",
    "ownership-does-not-revert",
    "actor-and-outcome-attributed",
  ]),
  functionalCase("shared.concurrency", "shared", [
    "same-tab-reads-concurrent",
    "same-tab-mutations-serialized",
    "different-tab-mutations-concurrent",
  ]),
  functionalCase("fault.relay-loss", "fault", [
    "fault-trigger-recorded",
    "bounded-reconnect",
    "no-duplicate-action-or-provider-run",
  ]),
  functionalCase("fault.controller-crash", "fault", [
    "fault-trigger-recorded",
    "health-degrades-clearly",
    "recovery-does-not-duplicate-tabs-or-authority",
  ]),
  functionalCase("fault.streamer-crash", "fault", [
    "fault-trigger-recorded",
    "viewer-degrades-without-blocking-kernel",
    "display-recovers-once",
  ]),
  functionalCase("fault.browser-crash", "fault", [
    "fault-trigger-recorded",
    "stale-references-rejected",
    "browser-recovers-with-one-tab-registry",
  ]),
  functionalCase("fault.stale-endpoint", "fault", [
    "stale-response-ignored",
    "current-selection-preserved",
    "retry-remains-bounded",
  ]),
  functionalCase("fault.queue-saturation", "fault", [
    "queue-limit-reached-deterministically",
    "terminal-readers-remain-live",
    "backpressure-is-bounded",
  ]),
  functionalCase("fault.memory-pressure", "fault", [
    "admission-closes-before-oom",
    "active-state-remains-consistent",
    "resource-recovery-recorded",
  ]),
  functionalCase("resource.safety", "resource", [
    "idle-and-active-process-residency-recorded",
    "cpu-disk-and-process-count-recorded",
    "headroom-gate-enforced-before-launch",
    "post-run-resource-delta-recorded",
  ]),
  functionalCase("cleanup.resources", "cleanup", [
    "owned-processes-stopped",
    "owned-containers-volumes-and-ports-removed",
    "temporary-state-removed",
    "retained-evidence-secret-scan-clean",
  ]),
])

const CASE_BY_ID = new Map(CASES.map((item) => [item.id, item]))
const ALL_CASE_IDS = deepFreeze(CASES.map((item) => item.id))
const DEFAULT_REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..")

export function browserComputerFunctionalCases() {
  return CASES
}

export function validateBrowserComputerFunctionalEvidence(report, {
  requiredCaseIds,
  repoRoots = [],
} = {}) {
  requireObject(report, "browser/computer functional evidence")
  if (report.schema !== BROWSER_COMPUTER_FUNCTIONAL_EVIDENCE_SCHEMA) {
    throw new Error(`browser/computer functional evidence has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  requireText(report.runId, "browser/computer functional evidence.runId")
  validateDrillTimestampOrder(report, "browser/computer functional evidence")
  requireProfile(report.profile)
  requireStack(report.stack)
  validateArtifactRoot(report.artifactRoot, [DEFAULT_REPO_ROOT, ...repoRoots])

  const declaredIds = validateRequiredCaseIds(report.caseIds, "declared browser/computer case ids")
  const expectedCoverage = JSON.stringify(declaredIds) === JSON.stringify(ALL_CASE_IDS) ? "complete" : "subset"
  if (report.coverage !== expectedCoverage) {
    throw new Error(`browser/computer functional evidence.coverage must be ${expectedCoverage}`)
  }
  const requiredIds = requiredCaseIds === undefined
    ? declaredIds
    : validateRequiredCaseIds(requiredCaseIds, "required browser/computer case ids")
  if (JSON.stringify(declaredIds) !== JSON.stringify(requiredIds)) {
    throw new Error(`browser/computer functional evidence declared coverage differs; expected ${requiredIds.join(", ")}`)
  }
  if (!Array.isArray(report.cases)) throw new Error("browser/computer functional evidence.cases is not an array")
  const actualIds = report.cases.map((result, index) => {
    requireObject(result, `browser/computer functional evidence.cases[${index}]`)
    requireText(result.caseId, `browser/computer functional evidence.cases[${index}].caseId`)
    return result.caseId
  })
  if (new Set(actualIds).size !== actualIds.length) {
    throw new Error("browser/computer functional evidence has duplicate case ids")
  }
  if (JSON.stringify(actualIds) !== JSON.stringify(requiredIds)) {
    throw new Error(`browser/computer functional evidence case order or coverage differs; expected ${requiredIds.join(", ")}`)
  }

  for (const [index, result] of report.cases.entries()) {
    validateCaseResult(result, CASE_BY_ID.get(result.caseId), index)
  }
  const expectedStatus = report.cases.every((result) => result.status === "passed") ? "passed" : "failed"
  if (report.status !== expectedStatus) {
    throw new Error(`browser/computer functional evidence.status must be ${expectedStatus}`)
  }
  if (report.ok !== (report.status === "passed")) {
    throw new Error("browser/computer functional evidence.ok does not match status")
  }
  assertNoSecrets(report, "browser/computer functional evidence")
  return report
}

function validateCaseResult(result, definition, index) {
  const source = `browser/computer functional evidence.cases[${index}]`
  if (!definition) throw new Error(`${source}.caseId is unknown`)
  if (!["passed", "failed", "not_applicable"].includes(result.status)) {
    throw new Error(`${source}.status is invalid`)
  }
  if (result.status === "not_applicable") {
    requireText(result.reason, `${source}.reason`)
    if (result.assertions !== undefined && (!Array.isArray(result.assertions) || result.assertions.length > 0)) {
      throw new Error(`${source}.assertions must be empty when not applicable`)
    }
    return
  }
  if (!Array.isArray(result.assertions)) throw new Error(`${source}.assertions is not an array`)
  const actualAssertionIds = result.assertions.map((assertion, assertionIndex) => {
    requireObject(assertion, `${source}.assertions[${assertionIndex}]`)
    requireText(assertion.id, `${source}.assertions[${assertionIndex}].id`)
    if (typeof assertion.ok !== "boolean") throw new Error(`${source}.assertions[${assertionIndex}].ok is not boolean`)
    if (assertion.evidence === undefined || assertion.evidence === null || assertion.evidence === "") {
      throw new Error(`${source}.assertions[${assertionIndex}].evidence is missing`)
    }
    return assertion.id
  })
  if (JSON.stringify(actualAssertionIds) !== JSON.stringify(definition.assertions)) {
    throw new Error(`${source}.assertions do not match ${definition.id}`)
  }
  const assertionsPassed = result.assertions.every((assertion) => assertion.ok)
  if ((result.status === "passed") !== assertionsPassed) {
    throw new Error(`${source}.status does not match its assertions`)
  }
}

function validateRequiredCaseIds(requiredCaseIds, source) {
  if (!Array.isArray(requiredCaseIds) || requiredCaseIds.length === 0) {
    throw new Error(`${source} must be a non-empty array`)
  }
  if (new Set(requiredCaseIds).size !== requiredCaseIds.length) {
    throw new Error(`${source} contain duplicates`)
  }
  for (const caseId of requiredCaseIds) {
    if (!CASE_BY_ID.has(caseId)) throw new Error(`${source} contain unknown case id ${caseId}`)
  }
  const catalogIndexes = requiredCaseIds.map((caseId) => ALL_CASE_IDS.indexOf(caseId))
  if (catalogIndexes.some((value, index) => index > 0 && value <= catalogIndexes[index - 1])) {
    throw new Error(`${source} must follow catalog order`)
  }
  return requiredCaseIds
}

function requireProfile(profile) {
  requireObject(profile, "browser/computer functional evidence.profile")
  for (const field of ["id", "platform", "topology"]) {
    requireText(profile[field], `browser/computer functional evidence.profile.${field}`)
  }
}

function requireStack(stack) {
  requireObject(stack, "browser/computer functional evidence.stack")
  for (const field of [
    "display",
    "displayVersion",
    "browserControl",
    "browserControlVersion",
    "browser",
    "browserVersion",
  ]) {
    requireText(stack[field], `browser/computer functional evidence.stack.${field}`)
  }
}

function validateArtifactRoot(artifactRoot, repoRoots) {
  requireText(artifactRoot, "browser/computer functional evidence.artifactRoot")
  if (!path.isAbsolute(artifactRoot)) {
    throw new Error("browser/computer functional evidence.artifactRoot must be absolute")
  }
  const resolved = path.resolve(artifactRoot)
  for (const repoRoot of repoRoots) {
    const relative = path.relative(path.resolve(repoRoot), resolved)
    if (relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative))) {
      throw new Error(`browser/computer functional evidence.artifactRoot must stay outside repositories: ${resolved}`)
    }
  }
}

function functionalCase(id, category, assertions) {
  return { id, category, assertions }
}

function requireObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
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

function deepFreeze(value) {
  if (!value || typeof value !== "object" || Object.isFrozen(value)) return value
  Object.freeze(value)
  for (const child of Object.values(value)) deepFreeze(child)
  return value
}
