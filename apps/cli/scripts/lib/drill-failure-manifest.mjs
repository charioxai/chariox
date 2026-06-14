import { readFile, stat } from "node:fs/promises"
import path from "node:path"
import {
  countDrillAggregateEntriesBy,
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import { classifyDrillChildFailure } from "./drill-child-process.mjs"
import {
  drillFailureClassificationForKind,
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
  isKnownDrillFailureClassification,
} from "./drill-failure-taxonomy.mjs"
import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
  redactDrillSecretText,
} from "./drill-secrets.mjs"
import { drillRuntimeSignalOwnerCounts } from "./drill-runtime-signals.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"

const FAILURE_MANIFEST_FILE = "arroba-drill-failure.json"
const FAILURE_MANIFEST_SCHEMA = "arroba.drill.failure.v1"

export async function readDrillFailureManifest(inputPath) {
  const manifestPath = await resolveFailureManifestPath(inputPath)
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"))
  validateDrillFailureManifest(manifest, manifestPath)
  return manifest
}

export async function findDrillFailureManifestPaths(rootPaths, { maxDepth = 8 } = {}) {
  return await findDrillJsonArtifactPaths(rootPaths, {
    fileName: FAILURE_MANIFEST_FILE,
    maxDepth,
  })
}

export async function resolveFailureManifestPath(inputPath) {
  const inputStat = await stat(inputPath)
  return inputStat.isDirectory() ? path.join(inputPath, FAILURE_MANIFEST_FILE) : inputPath
}

export function validateDrillFailureManifest(manifest, source = "manifest") {
  if (!manifest || typeof manifest !== "object") {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== FAILURE_MANIFEST_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!nonEmptyString(manifest.rootDir)) {
    throw new Error(`${source} is missing rootDir`)
  }
  if (!nonEmptyString(manifest.failedAt)) {
    throw new Error(`${source} is missing failedAt`)
  }
  parseDrillIsoTimestamp(manifest.failedAt, `${source}.failedAt`)
  if (!manifest.metadata || typeof manifest.metadata !== "object" || Array.isArray(manifest.metadata)) {
    throw new Error(`${source} has invalid metadata`)
  }
  validateFailureMetadataValue(manifest.metadata, `${source}.metadata`)
  if (manifest.error !== null) {
    validateFailureError(manifest.error, `${source}.error`)
  }
}

export function summarizeDrillFailureManifest(manifest, { source = null } = {}) {
  validateDrillFailureManifest(manifest, source ?? "manifest")
  const classification = classifyDrillFailureManifest(manifest)
  return {
    schema: manifest.schema,
    source,
    rootDir: manifest.rootDir,
    failedAt: manifest.failedAt,
    metadata: summarizeMetadata(manifest.metadata),
    error: manifest.error
      ? {
          name: manifest.error.name,
          message: redactDrillSecretText(manifest.error.message),
          hasStack: typeof manifest.error.stack === "string" && manifest.error.stack.length > 0,
        }
      : null,
    classification,
  }
}

export function formatDrillFailureManifestSummary(manifest, { source = null } = {}) {
  const summary = summarizeDrillFailureManifest(manifest, { source })
  const drill = summary.metadata.drill ?? "unknown"
  const lines = [
    `drill failure: ${drill}${source ? ` (${source})` : ""}`,
    `root=${summary.rootDir}`,
    `failed_at=${summary.failedAt}`,
  ]
  const metadata = Object.entries(summary.metadata)
    .filter(([key]) => key !== "drill")
    .map(([key, value]) => `${key}=${value}`)
  if (metadata.length > 0) {
    lines.push(`metadata: ${metadata.join(" ")}`)
  }
  if (summary.error) {
    lines.push(`error=${summary.error.name}: ${summary.error.message}`)
    lines.push(`stack=${summary.error.hasStack ? "preserved" : "absent"}`)
  } else {
    lines.push("error=none")
  }
  lines.push(`owner=${summary.classification.owner} classification=${summary.classification.kind}`)
  lines.push(`next: ${summary.classification.nextAction}`)
  return lines.join("\n")
}

export function summarizeDrillFailureManifests(manifests, { sources = [] } = {}) {
  const summaries = manifests.map((manifest, index) => summarizeDrillFailureManifest(manifest, {
    source: sources[index] ?? null,
  }))
  const owners = new Map()
  const classifications = new Map()
  const nextActions = new Map()
  const runtimeSignals = new Map()
  const failures = []
  for (const summary of summaries) {
    const owner = summary.classification.owner
    const kind = summary.classification.kind
    const nextAction = summary.classification.nextAction
    const failureRuntimeSignals = runtimeSignalsFromMetadata(summary.metadata)
    owners.set(owner, (owners.get(owner) ?? 0) + 1)
    classifications.set(kind, (classifications.get(kind) ?? 0) + 1)
    countDrillAggregateNextAction(nextActions, { owner, classification: kind, nextAction })
    countRuntimeSignals(runtimeSignals, failureRuntimeSignals)
    failures.push({
      drill: summary.metadata.drill ?? "unknown",
      source: summary.source,
      rootDir: summary.rootDir,
      owner,
      classification: kind,
      runtimeSignals: failureRuntimeSignals,
      nextAction,
    })
  }
  return {
    schema: "arroba.drill.failure.aggregate.v1",
    total: summaries.length,
    owners: Object.fromEntries([...owners.entries()].sort(([left], [right]) => left.localeCompare(right))),
    classifications: Object.fromEntries([...classifications.entries()].sort(([left], [right]) => left.localeCompare(right))),
    runtimeSignals: Object.fromEntries([...runtimeSignals.entries()].sort(([left], [right]) => left.localeCompare(right))),
    nextActions: formatDrillAggregateNextActionCounts(nextActions),
    failures,
  }
}

export function formatDrillFailureManifestAggregateSummary(aggregate) {
  validateDrillFailureManifestAggregate(aggregate)
  const lines = [
    "drill failure aggregate:",
    `total=${aggregate.total}`,
  ]
  const owners = Object.entries(aggregate.owners)
  if (owners.length > 0) {
    lines.push(`owners: ${owners.map(([owner, count]) => `${owner}=${count}`).join(" ")}`)
  }
  const classifications = Object.entries(aggregate.classifications)
  if (classifications.length > 0) {
    lines.push(`classifications: ${classifications.map(([kind, count]) => `${kind}=${count}`).join(" ")}`)
  }
  const runtimeSignals = Object.entries(aggregate.runtimeSignals ?? {})
  if (runtimeSignals.length > 0) {
    lines.push(`runtime_signals: ${runtimeSignals.map(([signal, count]) => `${signal}=${count}`).join(" ")}`)
    lines.push(`runtime_signal_owners: ${formatCountObject(drillRuntimeSignalOwnerCounts(aggregate.runtimeSignals))}`)
  }
  if (Array.isArray(aggregate.nextActions) && aggregate.nextActions.length > 0) {
    lines.push("next actions:")
    for (const action of aggregate.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.nextAction}`)
    }
  }
  if (aggregate.failures.length > 0) {
    lines.push("failures:")
    for (const failure of aggregate.failures) {
      const source = failure.source ? ` source=${failure.source}` : ""
      const runtimeSignals = failure.runtimeSignals?.length > 0
        ? ` runtime_signals=${failure.runtimeSignals.join(",")}`
        : ""
      lines.push(`- ${failure.drill} owner=${failure.owner} classification=${failure.classification}${runtimeSignals} root=${failure.rootDir}${source}`)
      lines.push(`  next: ${failure.nextAction}`)
    }
  }
  return lines.join("\n")
}

export function classifyDrillFailureManifest(manifest) {
  const errorText = [
    manifest.error?.name,
    manifest.error?.message,
    manifest.error?.stack,
  ].filter(Boolean).join("\n")
  const childClassification = classifyDrillChildFailure(errorText)
  if (childClassification !== "child-process") {
    return drillFailureClassificationForKind(childClassification, { target: "drill", rootDir: manifest.rootDir })
  }
  if (/docker|colima/i.test(errorText)) {
    return drillFailureClassificationForKind("docker-runtime", { target: "drill", rootDir: manifest.rootDir })
  }
  if (/relay|websocket|connection reset|target.*stale|target.*offline/i.test(errorText)) {
    return drillFailureClassificationForKind("relay-runtime", { target: "drill", rootDir: manifest.rootDir })
  }
  return drillFailureClassificationForKind(childClassification, { target: "drill", rootDir: manifest.rootDir })
}

export function validateDrillFailureManifestAggregate(aggregate) {
  if (!aggregate || typeof aggregate !== "object") {
    throw new Error("aggregate is not an object")
  }
  if (aggregate.schema !== "arroba.drill.failure.aggregate.v1") {
    throw new Error(`aggregate has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  if (!Number.isFinite(aggregate.total) || aggregate.total < 0) {
    throw new Error("aggregate has invalid total")
  }
  if (!aggregate.owners || typeof aggregate.owners !== "object" || Array.isArray(aggregate.owners)) {
    throw new Error("aggregate has invalid owners")
  }
  if (!aggregate.classifications || typeof aggregate.classifications !== "object" || Array.isArray(aggregate.classifications)) {
    throw new Error("aggregate has invalid classifications")
  }
  if (!aggregate.runtimeSignals || typeof aggregate.runtimeSignals !== "object" || Array.isArray(aggregate.runtimeSignals)) {
    throw new Error("aggregate has invalid runtimeSignals")
  }
  if (!Array.isArray(aggregate.failures)) {
    throw new Error("aggregate has invalid failures")
  }
  if (aggregate.nextActions !== undefined && !Array.isArray(aggregate.nextActions)) {
    throw new Error("aggregate has invalid nextActions")
  }
  for (const [index, action] of (aggregate.nextActions ?? []).entries()) {
    validateDrillAggregateNextAction(action, `aggregate.nextActions[${index}]`)
  }
  for (const [index, failure] of aggregate.failures.entries()) {
    validateDrillFailureAggregateEntry(failure, `aggregate.failures[${index}]`)
  }
  validateDrillFailureAggregateConsistency(aggregate)
}

function validateDrillFailureAggregateConsistency(aggregate) {
  if (aggregate.total !== aggregate.failures.length) {
    throw new Error("aggregate total does not match failures")
  }
  assertObjectCountsMatchEntries("aggregate owners", aggregate.owners, aggregate.failures, "owner")
  assertObjectCountsMatchEntries("aggregate classifications", aggregate.classifications, aggregate.failures, "classification")
  assertRuntimeSignalCountsMatchFailures(aggregate)
  assertNextActionCountsMatchFailures(aggregate)
}

function assertObjectCountsMatchEntries(label, counts, entries, key) {
  const expected = countDrillAggregateEntriesBy(entries, (entry) => entry[key])
  if (JSON.stringify(counts) !== JSON.stringify(expected)) {
    throw new Error(`${label} do not match failures`)
  }
}

function assertNextActionCountsMatchFailures(aggregate) {
  const expected = new Map()
  for (const failure of aggregate.failures) {
    countDrillAggregateNextAction(expected, {
      owner: failure.owner,
      classification: failure.classification,
      nextAction: failure.nextAction,
    })
  }
  const expectedActions = formatDrillAggregateNextActionCounts(expected)
  if (JSON.stringify(aggregate.nextActions ?? []) !== JSON.stringify(expectedActions)) {
    throw new Error("aggregate nextActions do not match failures")
  }
}

function assertRuntimeSignalCountsMatchFailures(aggregate) {
  const expected = new Map()
  for (const failure of aggregate.failures) {
    countRuntimeSignals(expected, failure.runtimeSignals ?? [])
  }
  const expectedSignals = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.runtimeSignals ?? {}) !== JSON.stringify(expectedSignals)) {
    throw new Error("aggregate runtimeSignals do not match failures")
  }
}

function validateDrillFailureAggregateEntry(failure, source) {
  if (!failure || typeof failure !== "object" || Array.isArray(failure)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["drill", "rootDir", "owner", "classification", "nextAction"]) {
    if (!nonEmptyString(failure[key])) {
      throw new Error(`${source} is missing ${key}`)
    }
  }
  if (!isKnownDrillFailureClassification(failure.classification)) {
    throw new Error(`${source} has unknown classification ${JSON.stringify(failure.classification)}`)
  }
  const expectedOwner = drillFailureOwnerForClassification(failure.classification)
  if (failure.owner !== expectedOwner) {
    throw new Error(`${source} owner ${JSON.stringify(failure.owner)} does not match classification ${JSON.stringify(failure.classification)}`)
  }
  const expectedNextAction = drillFailureNextActionForClassification(failure.classification, { target: "drill", rootDir: failure.rootDir })
  if (failure.nextAction !== expectedNextAction) {
    throw new Error(`${source} nextAction does not match classification ${JSON.stringify(failure.classification)}`)
  }
  if (failure.source !== null && failure.source !== undefined && !nonEmptyString(failure.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (!Array.isArray(failure.runtimeSignals)) {
    throw new Error(`${source} has invalid runtimeSignals`)
  }
  for (const [index, signal] of failure.runtimeSignals.entries()) {
    if (!nonEmptyString(signal)) {
      throw new Error(`${source}.runtimeSignals[${index}] is invalid`)
    }
  }
}

function validateFailureError(error, source) {
  if (!error || typeof error !== "object" || Array.isArray(error)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(error.name)) {
    throw new Error(`${source} is missing name`)
  }
  if (typeof error.message !== "string") {
    throw new Error(`${source} has invalid message`)
  }
  if (error.stack !== null && typeof error.stack !== "string") {
    throw new Error(`${source} has invalid stack`)
  }
}

function validateFailureMetadataValue(value, source) {
  if (typeof value === "string") {
    if (looksLikeDrillSecretValue(value)) {
      throw new Error(`${source} includes secret-looking metadata value`)
    }
    return
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return
  if (Array.isArray(value)) {
    for (const [index, item] of value.entries()) {
      validateFailureMetadataValue(item, `${source}[${index}]`)
    }
    return
  }
  if (!value || typeof value !== "object") {
    throw new Error(`${source} has unsupported metadata value`)
  }
  for (const [key, childValue] of Object.entries(value)) {
    validateFailureMetadataValue(childValue, `${source}.${key}`)
  }
}

function summarizeMetadata(metadata) {
  const entries = Object.entries(metadata)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => [key, summarizeMetadataValue(key, value)])
    .filter(([, value]) => value !== null)
  return Object.fromEntries(entries)
}

function summarizeMetadataValue(key, value) {
  if (isSensitiveDrillKey(key)) return "<redacted>"
  if (typeof value === "string") return value.length > 160 ? `${value.slice(0, 157)}...` : value
  if (typeof value === "number" || typeof value === "boolean") return String(value)
  if (value === null) return "null"
  return null
}

function runtimeSignalsFromMetadata(metadata) {
  const value = metadata.runtimeSignals
  if (typeof value !== "string") return []
  return [...new Set(value.split(",").map((signal) => signal.trim()).filter(nonEmptyString))].sort()
}

function formatCountObject(counts) {
  return Object.entries(counts).map(([key, count]) => `${key}=${count}`).join(" ")
}

function countRuntimeSignals(target, signals) {
  for (const signal of signals) {
    target.set(signal, (target.get(signal) ?? 0) + 1)
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
