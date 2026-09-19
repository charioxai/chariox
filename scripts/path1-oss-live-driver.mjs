#!/usr/bin/env node

import { execFile as execFileCallback, spawn as spawnChild } from "node:child_process"
import { access, readFile, rename, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"

const execFile = promisify(execFileCallback)

export const PATH1_OSS_LIVE_DRIVER_SCHEMA = "chariox.path1.live-functional-driver-request.v1"
export const PATH1_OSS_LIVE_DRIVER_JOURNAL_SCHEMA = "chariox.path1.oss-live-driver-journal.v1"
export const PATH1_OSS_LIVE_RECEIPT_SCHEMA = "chariox.path1.oss-live-driver-receipt.v1"
export const PATH1_OSS_LIVE_SOURCE = "deployed-oss-live-drill"
export const PATH1_OSS_LIVE_APPLY_CONFIRMATION = "APPLY_PATH1_OSS_LIVE"
export const PATH1_PROTOCOLS = Object.freeze({ localDaemon: 333, relayPeer: 55 })
export const PATH1_REQUIRED_PROVIDERS = Object.freeze(["codex", "opencode", "claude"])
export const PATH1_REQUIRED_SURFACES = Object.freeze([
  "live-external-provider-live-parity-drill.mjs",
  "live-native-provider-tui-matrix-drill.mjs",
  "live-room-environment-pointer-click-drill.mjs",
  "live-room-takeover-reconnect-fault-drill.mjs",
])
export const PATH1_OFFICIAL_SURFACE_PATHS = Object.freeze({
  providerParity: "apps/cli/scripts/live-external-provider-live-parity-drill.mjs",
  nativeTuiMatrix: "apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
  browserComputer: "apps/cli/scripts/live-room-environment-pointer-click-drill.mjs",
  takeoverReconnect: "apps/cli/scripts/live-room-takeover-reconnect-fault-drill.mjs",
})

const MAX_INPUT_BYTES = 2 * 1024 * 1024
const MAX_OUTPUT_BYTES = 2 * 1024 * 1024
const ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{2,127}$/u
const RUN_ID = /^[a-z][a-z0-9._:-]{2,80}$/u
const REVISION = /^[0-9a-f]{40}$/u
const DIGEST = /^sha256:[a-f0-9]{64}$/u
const ABSOLUTE_PATH = /^\//u
const SECRET_KEY = /(?:token|secret|password|passwd|api[_-]?key|private[_-]?key|authorization|cookie|credential(?!Handles))/iu
const SECRET_VALUE = /(?:bearer\s+\S+|-----begin [^-]*private key-----|(?:sk|ghp|github_pat|xox[baprs]|akia)[a-z0-9_-]{8,}|(?:token|secret|password|api[_-]?key)\s*[:=]\s*\S+)/iu
const BROAD_CLEANUP = /(?:docker\s+(?:system|container|volume|builder)\s+prune|\b(?:system|container|volume|builder)\s+prune\b|rm\s+-rf\s+(?:\/|~|\.))/iu
const REDACTED = /^(?:\[redacted(?:-[a-z-]+)?\]|<redacted>)$/u

export class Path1OssLiveDriverError extends Error {
  constructor(field, reason, code = "blocked") {
    super(`Path 1 OSS live driver blocked at ${field}: ${reason}`)
    this.name = "Path1OssLiveDriverError"
    this.field = field
    this.reason = reason
    this.code = code
  }
}

function fail(field, reason, code = "blocked") {
  throw new Path1OssLiveDriverError(field, reason, code)
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value))
}

function own(value, key) {
  return isRecord(value) && Object.hasOwn(value, key)
}

function requireRecord(value, field) {
  if (!isRecord(value)) fail(field, "must be an object")
  return value
}

function requireText(value, field) {
  if (typeof value !== "string" || value.trim() === "") fail(field, "must be non-empty text")
  return value
}

function requireId(value, field) {
  if (typeof value !== "string" || !ID.test(value)) fail(field, "must be a bounded identifier")
  return value
}

function requireRunId(value, field = "runId") {
  if (typeof value !== "string" || !RUN_ID.test(value)) fail(field, "must be a lowercase idempotent run ID")
  return value
}

function requireRevision(value, field) {
  if (typeof value !== "string" || !REVISION.test(value)) fail(field, "must be a lowercase 40-character source SHA")
  return value
}

function requireDigest(value, field) {
  if (typeof value !== "string" || !DIGEST.test(value)) fail(field, "must be an immutable sha256 digest")
  return value
}

function requireAbsolutePath(value, field) {
  if (typeof value !== "string" || !ABSOLUTE_PATH.test(value)) fail(field, "must be an absolute path")
  return path.resolve(value)
}

function nowTimestamp(now) {
  const value = now instanceof Date ? now : new Date(now)
  if (!Number.isFinite(value.valueOf())) fail("clock", "is invalid")
  return value.toISOString()
}

function redactedPath(value) {
  const base = path.basename(String(value ?? "journal.json")) || "journal.json"
  return `[redacted]/${base}`
}

function assertSanitized(value, label = "document", key = "", seen = new Set()) {
  if (typeof value === "string") {
    if (REDACTED.test(value)) return value
    if (SECRET_KEY.test(key) || SECRET_VALUE.test(value)) fail(label, "contains inline secret material")
    if (BROAD_CLEANUP.test(value)) fail(label, "contains broad cleanup")
    if (value.length > 200_000) fail(label, "contains an oversized text value")
    return value
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return value
  if (typeof value !== "object") fail(label, "contains an unsupported value")
  if (seen.has(value)) fail(label, "contains a cyclic value")
  seen.add(value)
  if (Array.isArray(value)) value.forEach((child, index) => assertSanitized(child, `${label}[${index}]`, key, seen))
  else for (const [childKey, child] of Object.entries(value)) assertSanitized(child, `${label}.${childKey}`, childKey, seen)
  seen.delete(value)
  return value
}

function safeFailure(error) {
  if (error instanceof Path1OssLiveDriverError) {
    return { code: error.code, field: error.field, reason: error.reason }
  }
  if (error?.code === "interrupted") {
    return { code: "interrupted", field: null, reason: "live seam execution was interrupted" }
  }
  return { code: "live-driver-failed", field: null, reason: "live seam execution failed" }
}

function publicationReceiptFrom(input) {
  const candidates = [
    ["publicationReceipt", input?.publicationReceipt],
    ["runnerContext.publicationReceipt", input?.runnerContext?.publicationReceipt],
    ["journal.publicationReceipt", input?.journal?.publicationReceipt],
  ].filter(([, value]) => value !== undefined && value !== null)
  if (candidates.length === 0) fail("publicationReceipt", "the exact reviewed publication receipt is required")
  const [field, receipt] = candidates[0]
  assertSanitized(receipt, field)
  return { field, receipt: requireRecord(receipt, field) }
}

function sourceHeadFromPublicationReceipt(receipt) {
  if (receipt.schema !== "chariox.path1.oss-publication-receipt.v1") {
    fail("publicationReceipt.schema", "is not the reviewed OSS publication receipt schema")
  }
  if (receipt.status !== "published") fail("publicationReceipt.status", "must be published")
  const source = requireRecord(receipt.source, "publicationReceipt.source")
  if (source.treeClean !== true) fail("publicationReceipt.source.treeClean", "must be true")
  const candidates = [
    ["source.ossRevision", source.ossRevision],
    ["sourceRevision", receipt.sourceRevision],
    ["ossRevision", receipt.ossRevision],
  ].filter(([, value]) => value !== undefined && value !== null)
  if (candidates.length === 0) fail("publicationReceipt.source.ossRevision", "must be present")
  const values = new Set(candidates.map(([, value]) => String(value)))
  if (values.size !== 1) fail("publicationReceipt.source.ossRevision", "conflicting source heads are present")
  const sourceHead = requireRevision(candidates[0][1], `publicationReceipt.${candidates[0][0]}`)
  if (source.revisionLabel?.value !== sourceHead) fail("publicationReceipt.source.revisionLabel", "must bind the source head")
  const protocols = requireRecord(receipt.protocols, "publicationReceipt.protocols")
  if (protocols.localDaemon !== PATH1_PROTOCOLS.localDaemon || protocols.relayPeer !== PATH1_PROTOCOLS.relayPeer) {
    fail("publicationReceipt.protocols", "must use the supported exact protocol pair")
  }
  const image = requireRecord(receipt.image, "publicationReceipt.image")
  requireDigest(image.digest, "publicationReceipt.image.digest")
  if (image.signatureVerification !== "verified" || image.attestationVerification !== "verified") {
    fail("publicationReceipt.image", "signature and attestation verification are required")
  }
  const manifests = Array.isArray(image.manifests) ? image.manifests : []
  if (manifests.length !== 1 || manifests[0].platform !== "linux/amd64" || manifests[0].digest !== image.digest) {
    fail("publicationReceipt.image.manifests", "must contain one immutable linux/amd64 manifest")
  }
  if (receipt.publication?.status !== "succeeded" || receipt.publication?.quarantine !== "none") {
    fail("publicationReceipt.publication", "must be succeeded and quarantine-free")
  }
  return sourceHead
}

function validateContract(input, sourceHead) {
  if (input.schema !== PATH1_OSS_LIVE_DRIVER_SCHEMA) fail("schema", "is unsupported")
  const contract = requireRecord(input.contract, "contract")
  if (contract.ossRevision !== sourceHead) fail("contract.ossRevision", "does not match the publication receipt source head")
  if (contract.localDaemonProtocolVersion !== PATH1_PROTOCOLS.localDaemon
    || contract.relayPeerProtocolVersion !== PATH1_PROTOCOLS.relayPeer) {
    fail("contract.protocols", "do not match the reviewed release")
  }
  if (input.sourceHead !== undefined && input.sourceHead !== sourceHead) fail("sourceHead", "does not match the publication receipt")
  return contract
}

function resolveRunnerContext(input, sourceHead) {
  const context = requireRecord(input.runnerContext ?? input.context, "runnerContext")
  const runId = requireRunId(input.runId ?? context.runId)
  if (context.runId !== undefined && context.runId !== runId) fail("runnerContext.runId", "does not match the request")
  const allocationId = requireId(input.allocationId ?? context.allocationId, "allocationId")
  const machineId = requireId(input.machineId ?? context.machineId, "machineId")
  const homeKernelId = requireId(input.homeKernelId ?? context.homeKernelId, "homeKernelId")
  const relayRealmId = requireId(input.homeRelayRealmId ?? context.homeRelayRealmId, "homeRelayRealmId")
  const repoRoot = requireAbsolutePath(input.repoRoot ?? context.repoRoot ?? process.cwd(), "repoRoot")
  const evidenceRoot = context.evidenceRoot ?? input.evidenceRoot
  if (evidenceRoot !== undefined) requireAbsolutePath(evidenceRoot, "runnerContext.evidenceRoot")
  if (context.journalPath !== undefined) requireAbsolutePath(context.journalPath, "runnerContext.journalPath")
  return {
    ...context,
    runId,
    allocationId,
    machineId,
    homeKernelId,
    relayRealmId,
    repoRoot,
    sourceHead,
    sessionId: input.sessionId ?? context.sessionId ?? null,
    roomId: input.roomId ?? context.roomId ?? null,
  }
}

async function inspectSource(repoRoot, sourceHead, inspect = null) {
  const result = inspect
    ? await inspect({ repoRoot, sourceHead })
    : await (async () => {
      const head = await execFile("git", ["rev-parse", "HEAD"], { cwd: repoRoot, encoding: "utf8" })
      const status = await execFile("git", ["status", "--porcelain"], { cwd: repoRoot, encoding: "utf8" })
      return { head: head.stdout.trim(), dirty: status.stdout.trim() !== "" }
    })()
  if (result?.head !== sourceHead) fail("sourceHead", "the checked-out source is not the reviewed publication head")
  if (result?.dirty) fail("sourceTree", "must be clean before a live driver action")
  return { head: sourceHead, dirty: false }
}

async function existingOfficialSurfaces(repoRoot) {
  const result = {}
  for (const [name, relative] of Object.entries(PATH1_OFFICIAL_SURFACE_PATHS)) {
    const absolute = path.join(repoRoot, relative)
    try {
      await access(absolute)
    } catch {
      fail(`officialSurfaces.${name}`, `existing drill is absent: ${relative}`, "missing-product-seam")
    }
    result[name] = absolute
  }
  return result
}

function requiredPlanSteps({ providers = PATH1_REQUIRED_PROVIDERS } = {}) {
  return [
    ["validate-publication-receipt", "guard", false],
    ["verify-exact-source-head", "guard", false],
    ["journal-run-intent", "journal", true],
    ["bind-normal-kernel-relay-room", "oss-live-surface", true],
    ["transfer-context-through-home-kernel-and-relay", "oss-live-surface", true],
    ["wait-for-automatic-project-environment-ready", "oss-live-surface", false],
    ...providers.map((provider) => [`run-${provider}-official-live-cell`, "oss-live-surface", true]),
    ["run-browser-first-computer-fallback-capability-cells", "oss-live-surface", true],
    ["sample-live-resources", "oss-live-surface", false],
    ["journal-sanitized-live-evidence", "journal", true],
    ["emit-sanitized-per-cell-result", "receipt", true],
  ].map(([id, kind, mutation]) => ({ id, kind, mutation }))
}

export async function buildPath1OssLivePlan({ input, sourceInspector = null } = {}) {
  const source = requireRecord(input, "input")
  assertSanitized(source, "input")
  const { receipt } = publicationReceiptFrom(source)
  const sourceHead = sourceHeadFromPublicationReceipt(receipt)
  validateContract(source, sourceHead)
  const context = resolveRunnerContext(source, sourceHead)
  await inspectSource(context.repoRoot, sourceHead, sourceInspector)
  const surfaces = await existingOfficialSurfaces(context.repoRoot)
  return Object.freeze({
    schema: PATH1_OSS_LIVE_DRIVER_SCHEMA,
    mode: "plan",
    runId: context.runId,
    sourceHead,
    protocols: PATH1_PROTOCOLS,
    liveObserved: false,
    dryRun: true,
    sourceTestOnly: true,
    sameRoomRequired: true,
    seam: "runner-context-capable-same-room-adapter-required-for-apply",
    officialSurfaces: surfaces,
    steps: requiredPlanSteps(),
  })
}

function seamFor(input, action, provider) {
  const seams = input.runnerContext?.seams ?? input.seams ?? {}
  const candidates = provider
    ? [seams.providers?.[provider], seams.providerCell, seams[action]]
    : [seams[action], seams.liveRoom, seams.driver]
  const command = candidates.find((value) => typeof value === "string" && value.trim())
  if (!command) fail(`seams.${action}`, "a runner-context-capable same-Room adapter is required", "missing-product-seam")
  return requireAbsolutePath(command, `seams.${action}`)
}

function validateSurfaceManifest(input) {
  const manifest = requireRecord(input.runnerContext?.surfaceManifest, "runnerContext.surfaceManifest")
  for (const field of ["sameRoom", "acceptsRunnerContext", "normalKernelAuthority", "relayTransportOnly"]) {
    if (manifest[field] !== true) fail(`runnerContext.surfaceManifest.${field}`, "must be true for apply", "missing-product-seam")
  }
  return manifest
}

function basePayload(input, context, sourceHead, action, provider, surfaces) {
  const payload = {
    schema: PATH1_OSS_LIVE_DRIVER_SCHEMA,
    action,
    provider: provider ?? null,
    contract: input.contract,
    runId: context.runId,
    sourceHead,
    allocationId: context.allocationId,
    machineId: context.machineId,
    homeKernelId: context.homeKernelId,
    homeRelayRealmId: context.relayRealmId,
    sessionId: context.sessionId,
    roomId: context.roomId,
    contextPlan: input.contextPlan ?? context.contextPlan ?? null,
    authority: {
      kernel: "normal-kernel",
      relay: "transport-only",
      room: "kernel-owned",
    },
    officialSurfaces: surfaces,
    providerHarness: provider ? { provider, official: true, harness: provider } : null,
    idempotencyKey: `path1:${context.runId}:${action}:${provider ?? "none"}`,
  }
  assertSanitized(payload, "driverPayload")
  return payload
}

function validateLiveBase(value, action, context, sourceHead) {
  const result = requireRecord(value, `live.${action}`)
  assertSanitized(result, `live.${action}`)
  if (result.source !== PATH1_OSS_LIVE_SOURCE || result.liveObserved !== true
    || result.dryRun !== false || result.sourceTestOnly !== false) {
    fail(`live.${action}`, "must be observed from the deployed live OSS surface")
  }
  if (result.action !== action || result.status === "failed" || result.ok === false) {
    fail(`live.${action}`, "reported a failed or mismatched action")
  }
  if (result.runId !== context.runId || result.allocationId !== context.allocationId
    || result.machineId !== context.machineId) {
    fail(`live.${action}`, "does not bind to the guarded allocation")
  }
  if (result.sourceHead !== undefined && result.sourceHead !== sourceHead) fail(`live.${action}.sourceHead`, "is stale")
  requireId(result.receiptId, `live.${action}.receiptId`)
  return result
}

function validateRoomBinding(result, context, field) {
  requireId(context.sessionId, `${field}.sessionId`)
  requireId(context.roomId, `${field}.roomId`)
  if (result.sessionId !== context.sessionId || result.roomId !== context.roomId) {
    fail(field, "does not bind to the same Room")
  }
}

function validateContextTransfer(value, context, input, sourceHead) {
  const result = validateLiveBase(value, "context-transfer", context, sourceHead)
  if (result.status !== "completed" || result.kernelAuthoritative !== true || result.relayTransportOnly !== true) {
    fail("live.context-transfer", "must complete through kernel authority and relay transport")
  }
  requireId(result.sessionId, "live.context-transfer.sessionId")
  requireId(result.roomId, "live.context-transfer.roomId")
  const plan = input.contextPlan ?? context.contextPlan
  if (plan?.contextId && result.contextId !== plan.contextId) fail("live.context-transfer.contextId", "does not match the runner plan")
  if (plan?.planDigest && result.planDigest !== plan.planDigest) fail("live.context-transfer.planDigest", "does not match the runner plan")
  return result
}

function validateProjectReady(value, context, sourceHead) {
  const result = validateLiveBase(value, "project-environment-ready", context, sourceHead)
  if (result.status !== "ready" || result.automatic !== true || result.environmentId !== context.allocationId) {
    fail("live.project-environment-ready", "must report automatic Ready for the same allocation")
  }
  validateRoomBinding(result, context, "live.project-environment-ready")
  return result
}

function validateStructuredCell(value, field) {
  const cell = requireRecord(value, field)
  if (cell.structured !== true || cell.completed !== true) fail(field, "must complete structured Browser before Computer fallback")
  requireId(cell.receiptId, `${field}.receiptId`)
  return { structured: true, completed: true, receiptId: cell.receiptId }
}

function validateProviderCell(value, provider, context, sourceHead) {
  const result = validateLiveBase(value, "provider-cell", context, sourceHead)
  validateRoomBinding(result, context, `live.provider-cell.${provider}`)
  if (result.provider !== provider || result.providerId !== provider || result.official !== true
    || result.officialHarness !== provider) {
    fail(`live.provider-cell.${provider}`, "must use the requested official provider harness")
  }
  requireId(result.actorId, `live.provider-cell.${provider}.actorId`)
  requireId(result.threadId, `live.provider-cell.${provider}.threadId`)
  const completion = requireRecord(result.completion, `live.provider-cell.${provider}.completion`)
  if (completion.state !== "completed" || completion.exactlyOnce !== true || completion.count !== 1) {
    fail(`live.provider-cell.${provider}.completion`, "must complete exactly once")
  }
  const history = requireRecord(result.history, `live.provider-cell.${provider}.history`)
  if (history.durable !== true) fail(`live.provider-cell.${provider}.history`, "must be durable")
  requireId(history.receiptId, `live.provider-cell.${provider}.history.receiptId`)
  const surfaces = Array.isArray(result.surfaces) ? result.surfaces : []
  for (const surface of PATH1_REQUIRED_SURFACES.slice(0, 2)) {
    if (!surfaces.includes(surface)) fail(`live.provider-cell.${provider}.surfaces`, `must include ${surface}`)
  }
  return {
    ...result,
    provider,
    official: true,
    officialHarness: provider,
    actorId: result.actorId,
    threadId: result.threadId,
    browser: validateStructuredCell(result.browser, `live.provider-cell.${provider}.browser`),
    computerFallback: validateStructuredCell(result.computerFallback, `live.provider-cell.${provider}.computerFallback`),
    completion: { state: "completed", exactlyOnce: true, count: 1 },
    history: { durable: true, receiptId: history.receiptId },
    receiptId: result.receiptId,
  }
}

function requireTrue(value, field) {
  if (value !== true) fail(field, "must be true")
}

function validateCapabilities(value, providers, context, sourceHead) {
  const result = validateLiveBase(value, "capability-drills", context, sourceHead)
  validateRoomBinding(result, context, "live.capability-drills")
  const surfaces = Array.isArray(result.surfaces) ? result.surfaces : []
  for (const surface of PATH1_REQUIRED_SURFACES) if (!surfaces.includes(surface)) fail("live.capability-drills.surfaces", `must include ${surface}`)
  const topology = requireRecord(result.topology, "live.capability-drills.topology")
  const attachments = requireRecord(topology.attachments, "live.capability-drills.topology.attachments")
  for (const surface of ["web", "localTui", "remoteTui"]) {
    const attachment = requireRecord(attachments[surface], `live.capability-drills.topology.attachments.${surface}`)
    requireTrue(attachment.applicable, `live.capability-drills.topology.attachments.${surface}.applicable`)
    requireTrue(attachment.attached, `live.capability-drills.topology.attachments.${surface}.attached`)
    if (attachment.runId !== context.runId || attachment.sessionId !== context.sessionId || attachment.roomId !== context.roomId) {
      fail(`live.capability-drills.topology.attachments.${surface}`, "does not bind to the same Room")
    }
    requireId(attachment.receiptId, `live.capability-drills.topology.attachments.${surface}.receiptId`)
  }
  const readiness = requireRecord(topology.readiness, "live.capability-drills.topology.readiness")
  requireTrue(readiness.browserControllerReady, "live.capability-drills.topology.readiness.browserControllerReady")
  requireTrue(readiness.streamReady, "live.capability-drills.topology.readiness.streamReady")
  requireId(readiness.browserControllerReceiptId, "live.capability-drills.topology.readiness.browserControllerReceiptId")
  requireId(readiness.streamReceiptId, "live.capability-drills.topology.readiness.streamReceiptId")

  const functional = requireRecord(result.functional, "live.capability-drills.functional")
  const input = requireRecord(functional.input, "live.capability-drills.functional.input")
  const screenshotOcr = requireRecord(functional.screenshotOcr, "live.capability-drills.functional.screenshotOcr")
  for (const [name, item] of [
    ["pointer", input.pointer], ["keyboard", input.keyboard], ["clipboard", input.clipboard],
    ["screenshot", screenshotOcr.screenshot], ["ocr", screenshotOcr.ocr],
  ]) {
    requireTrue(item?.ok, `live.capability-drills.functional.${name}.ok`)
    if (item.runId !== context.runId || item.sessionId !== context.sessionId || item.roomId !== context.roomId) {
      fail(`live.capability-drills.functional.${name}`, "does not bind to the same Room")
    }
    requireId(item.receiptId, `live.capability-drills.functional.${name}.receiptId`)
  }
  const takeover = requireRecord(functional.takeover, "live.capability-drills.functional.takeover")
  for (const field of ["requested", "granted", "agentBlocked", "idempotentReplay", "sameRoom"]) requireTrue(takeover[field], `live.capability-drills.functional.takeover.${field}`)
  requireId(takeover.idempotencyKey, "live.capability-drills.functional.takeover.idempotencyKey")
  requireId(takeover.actionId, "live.capability-drills.functional.takeover.actionId")
  if (takeover.replayActionId !== takeover.actionId) fail("live.capability-drills.functional.takeover.replayActionId", "must be idempotent")
  requireId(takeover.receiptId, "live.capability-drills.functional.takeover.receiptId")
  const permission = requireRecord(functional.permission, "live.capability-drills.functional.permission")
  for (const field of ["requested", "interacted", "resolved"]) requireTrue(permission[field], `live.capability-drills.functional.permission.${field}`)
  if (!["approved", "denied"].includes(permission.outcome)) fail("live.capability-drills.functional.permission.outcome", "must be resolved")
  requireId(permission.receiptId, "live.capability-drills.functional.permission.receiptId")
  const reconnect = requireRecord(functional.reconnectReload, "live.capability-drills.functional.reconnectReload")
  for (const field of ["reconnected", "reloaded", "sameRoom", "sameSession", "providerThreadContinuity"]) requireTrue(reconnect[field], `live.capability-drills.functional.reconnectReload.${field}`)
  requireId(reconnect.receiptId, "live.capability-drills.functional.reconnectReload.receiptId")

  const history = requireRecord(result.history, "live.capability-drills.history")
  for (const field of ["durable", "sameRoom", "providerThreadsPersisted"]) requireTrue(history[field], `live.capability-drills.history.${field}`)
  if (!Number.isSafeInteger(history.entryCount) || history.entryCount < 1) fail("live.capability-drills.history.entryCount", "must be positive")
  requireId(history.receiptId, "live.capability-drills.history.receiptId")
  const lifecycle = requireRecord(result.saveRestartReconnect, "live.capability-drills.saveRestartReconnect")
  for (const field of ["saved", "restarted", "reconnected", "sameRoom", "sameSession", "providerThreadContinuity"]) requireTrue(lifecycle[field], `live.capability-drills.saveRestartReconnect.${field}`)
  requireId(lifecycle.receiptId, "live.capability-drills.saveRestartReconnect.receiptId")
  const threads = requireRecord(lifecycle.providerThreads, "live.capability-drills.saveRestartReconnect.providerThreads")
  for (const provider of providers) {
    const thread = requireRecord(threads[provider], `live.capability-drills.saveRestartReconnect.providerThreads.${provider}`)
    requireId(thread.before, `live.capability-drills.saveRestartReconnect.providerThreads.${provider}.before`)
    if (thread.before !== thread.after || thread.same !== true) fail(`live.capability-drills.saveRestartReconnect.providerThreads.${provider}`, "provider thread continuity is missing")
  }
  const idle = requireRecord(result.idle, "live.capability-drills.idle")
  if (!Number.isSafeInteger(idle.intervalMs) || idle.intervalMs < 1_000 || idle.sameProviderThreads !== true) fail("live.capability-drills.idle", "idle resume is not bounded")
  const resumed = requireRecord(idle.resumedRequest, "live.capability-drills.idle.resumedRequest")
  for (const field of ["completed", "exactlyOnce", "sameRoom"]) requireTrue(resumed[field], `live.capability-drills.idle.resumedRequest.${field}`)
  requireId(resumed.receiptId, "live.capability-drills.idle.resumedRequest.receiptId")
  const injected = requireRecord(result.failureInjection, "live.capability-drills.failureInjection")
  for (const field of ["injected", "errorObserved", "recovered", "bounded", "sameRoom", "noDuplicateCompletion"]) requireTrue(injected[field], `live.capability-drills.failureInjection.${field}`)
  requireText(injected.kind, "live.capability-drills.failureInjection.kind")
  if (!Number.isSafeInteger(injected.durationMs) || !Number.isSafeInteger(injected.limitMs) || injected.durationMs <= 0 || injected.durationMs > injected.limitMs) fail("live.capability-drills.failureInjection", "recovery is outside its bound")
  requireId(injected.receiptId, "live.capability-drills.failureInjection.receiptId")
  const resources = requireRecord(result.resources, "live.capability-drills.resources")
  const limits = requireRecord(resources.limits, "live.capability-drills.resources.limits")
  const samples = requireRecord(resources.samples, "live.capability-drills.resources.samples")
  for (const metric of ["vmBootMs", "browserReadyMs", "streamReadyMs", "screenshotOcrMs", "resumedRequestMs", "rssMb", "diskDeltaMb"]) {
    if (!Number.isFinite(limits[metric]) || limits[metric] <= 0 || !Number.isFinite(samples[metric]) || samples[metric] < 0 || samples[metric] > limits[metric]) fail(`live.capability-drills.resources.${metric}`, "is outside its declared bound")
  }
  requireTrue(resources.withinLimits, "live.capability-drills.resources.withinLimits")
  requireId(resources.receiptId, "live.capability-drills.resources.receiptId")
  return result
}

function stageFor(action, provider) {
  return provider ? `provider:${provider}` : action
}

function createJournal({ input, context, sourceHead, journalPath, now }) {
  return {
    schema: PATH1_OSS_LIVE_DRIVER_JOURNAL_SCHEMA,
    version: 1,
    status: "planned",
    sequence: 0,
    runId: context.runId,
    sourceHead,
    contract: cloneJson(input.contract),
    journal: { ref: `path1-oss-live-${context.runId}`, path: redactedPath(journalPath ?? "path1-oss-live.journal.json") },
    owned: { scope: "drill-owned", processes: [], artifacts: [] },
    stages: {},
    results: {},
    events: [],
    failure: null,
    updatedAt: nowTimestamp(now),
  }
}

function assertJournal(journal, context, sourceHead, journalPath) {
  requireRecord(journal, "journal")
  assertSanitized(journal, "journal")
  if (journal.schema !== PATH1_OSS_LIVE_DRIVER_JOURNAL_SCHEMA || journal.version !== 1) fail("journal", "schema is unsupported")
  if (journal.runId !== context.runId || journal.sourceHead !== sourceHead) fail("journal", "does not match the guarded run or source head")
  if (journal.journal?.path !== redactedPath(journalPath ?? "path1-oss-live.journal.json")) fail("journal.path", "does not match the guarded journal")
  if (journal.owned?.scope !== "drill-owned") fail("journal.owned.scope", "must be drill-owned")
  return journal
}

function updateJournal(journal, patch, phase, now) {
  const observedAt = nowTimestamp(typeof now === "function" ? now() : now)
  const next = {
    ...journal,
    ...patch,
    sequence: (Number.isSafeInteger(journal.sequence) ? journal.sequence : 0) + 1,
    updatedAt: observedAt,
    events: [...(journal.events ?? []), { phase, observedAt }],
  }
  assertSanitized(next, "journal")
  return next
}

export function createMemoryJournalStore(initial = null) {
  let current = initial ? cloneJson(initial) : null
  return {
    async load() { return current ? cloneJson(current) : null },
    async save(value) { current = cloneJson(value) },
    get value() { return current ? cloneJson(current) : null },
  }
}

export function createFileJournalStore(journalPath) {
  const absolute = requireAbsolutePath(journalPath, "journalPath")
  return {
    async load() {
      try {
        return JSON.parse(await readFile(absolute, "utf8"))
      } catch (error) {
        if (error?.code === "ENOENT") return null
        fail("journal", "could not be read safely")
      }
    },
    async save(value) {
      assertSanitized(value, "journal")
      const temporary = `${absolute}.${process.pid}.tmp`
      try {
        await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 })
        await rename(temporary, absolute)
      } catch {
        await writeFile(absolute, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 }).catch(() => {})
        fail("journal", "could not be written safely")
      }
    },
  }
}

async function invokeExternal(command, payload, { cwd, timeoutMs = 120_000, env = process.env } = {}) {
  const absolute = requireAbsolutePath(command, "seamCommand")
  const invocation = /\.(?:mjs|js)$/iu.test(absolute)
    ? { file: process.execPath, args: [absolute] }
    : { file: absolute, args: [] }
  return await new Promise((resolve, reject) => {
    const child = spawnChild(invocation.file, invocation.args, {
      cwd,
      env: { ...env, CHARIOX_PATH1_OSS_LIVE_DRIVER: "1" },
      stdio: ["pipe", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    const timeout = setTimeout(() => child.kill("SIGTERM"), timeoutMs)
    child.stdout.on("data", (chunk) => { stdout = `${stdout}${chunk}`.slice(-MAX_OUTPUT_BYTES) })
    child.stderr.on("data", (chunk) => { stderr = `${stderr}${chunk}`.slice(-MAX_OUTPUT_BYTES) })
    child.once("error", (error) => { clearTimeout(timeout); reject(error) })
    child.once("close", (code, signal) => {
      clearTimeout(timeout)
      if (code !== 0) {
        const error = new Path1OssLiveDriverError("seamCommand", "the real live seam exited unsuccessfully", signal ? "interrupted" : "live-seam-failed")
        error.exit = { code, signal }
        error.stderrObserved = Boolean(stderr)
        reject(error)
        return
      }
      let result
      try { result = JSON.parse(stdout) } catch { reject(new Path1OssLiveDriverError("seamCommand", "the real live seam did not return JSON", "live-seam-invalid-output")); return }
      try { assertSanitized(result, "seamResult") } catch (error) { reject(error); return }
      resolve(result)
    })
    child.stdin.end(`${JSON.stringify(payload)}\n`)
  })
}

export function createRealRunner({ cwd, env = process.env, timeoutMs = 120_000 } = {}) {
  return {
    kind: "real",
    async invoke({ command, payload }) {
      return invokeExternal(command, payload, { cwd, env, timeoutMs })
    },
    async cleanupOwned() {},
  }
}

async function executeAction({ input, context, sourceHead, action, provider, surfaces, runner, journal, store, now }) {
  const stage = stageFor(action, provider)
  if (journal.stages[stage] && journal.results[stage]) return { journal, result: journal.results[stage], resumed: true }
  const command = runner.kind === "injected" ? null : seamFor(input, action, provider)
  const artifactRoot = context.evidenceRoot ? requireAbsolutePath(context.evidenceRoot, "runnerContext.evidenceRoot") : null
  const artifactPath = artifactRoot ? redactedPath(path.join(artifactRoot, `${stage.replaceAll(":", "-")}.json`)) : "[redacted]/live-evidence.json"
  const processRecord = {
    id: `owned-${context.runId}-${stage.replaceAll(":", "-")}`,
    action,
    provider: provider ?? null,
    command: command ? redactedPath(command) : "[injected-test-runner]",
    status: "planned",
    pid: null,
    artifactPath,
  }
  const planned = updateJournal(journal, {
    status: "running",
    owned: {
      ...journal.owned,
      processes: [...journal.owned.processes, processRecord],
      artifacts: [...new Set([...journal.owned.artifacts, artifactPath])],
    },
  }, `${stage}-launch-planned`, now)
  await store.save(planned)
  const payload = basePayload(input, context, sourceHead, action, provider, surfaces)
  payload.artifactPath = artifactPath
  payload.runnerJournal = { ref: planned.journal.ref, sequence: planned.sequence }
  let result
  try {
    result = await runner.invoke({ action, provider, command, payload, journal: planned })
    if (action === "context-transfer") result = validateContextTransfer(result, context, input, sourceHead)
    else if (action === "project-environment-ready") result = validateProjectReady(result, context, sourceHead)
    else if (action === "provider-cell") result = validateProviderCell(result, provider, context, sourceHead)
    else if (action === "capability-drills") result = validateCapabilities(result, PATH1_REQUIRED_PROVIDERS, context, sourceHead)
    else fail("action", "is unsupported")
    const completedProcesses = planned.owned.processes.map((entry) => entry.id === processRecord.id ? { ...entry, status: "completed" } : entry)
    const completed = updateJournal(planned, {
      stages: { ...planned.stages, [stage]: true },
      results: { ...planned.results, [stage]: result },
      owned: { ...planned.owned, processes: completedProcesses },
    }, `${stage}-completed`, now)
    await store.save(completed)
    return { journal: completed, result, resumed: false }
  } catch (error) {
    const failedProcesses = planned.owned.processes.map((entry) => entry.id === processRecord.id ? { ...entry, status: "failed" } : entry)
    const failed = updateJournal(planned, {
      status: error?.code === "interrupted" ? "interrupted" : "failed",
      failure: safeFailure(error),
      owned: { ...planned.owned, processes: failedProcesses },
    }, `${stage}-${error?.code === "interrupted" ? "interrupted" : "failed"}`, now)
    await store.save(failed)
    return { journal: failed, result: null, failure: safeFailure(error), resumed: false }
  }
}

function resolveContextFromTransfer(context, result) {
  return {
    ...context,
    sessionId: result?.sessionId ?? context.sessionId,
    roomId: result?.roomId ?? context.roomId,
  }
}

export async function runPath1OssLiveAcceptance({
  input,
  mode = "plan",
  confirmed = false,
  runner = null,
  journalStore = null,
  sourceInspector = null,
  now = () => new Date(),
  cleanup = true,
} = {}) {
  const source = requireRecord(input, "input")
  assertSanitized(source, "input")
  const { receipt } = publicationReceiptFrom(source)
  const sourceHead = sourceHeadFromPublicationReceipt(receipt)
  validateContract(source, sourceHead)
  const initialContext = resolveRunnerContext(source, sourceHead)
  await inspectSource(initialContext.repoRoot, sourceHead, sourceInspector)
  const surfaces = await existingOfficialSurfaces(initialContext.repoRoot)
  const plan = {
    schema: PATH1_OSS_LIVE_DRIVER_SCHEMA,
    mode: "plan",
    runId: initialContext.runId,
    sourceHead,
    protocols: PATH1_PROTOCOLS,
    liveObserved: false,
    dryRun: true,
    sourceTestOnly: true,
    sameRoomRequired: true,
    seam: "runner-context-capable-same-room-adapter-required-for-apply",
    officialSurfaces: surfaces,
    steps: requiredPlanSteps(),
  }
  if (mode === "plan") return Object.freeze({ status: "plan", plan })
  if (mode !== "apply") fail("mode", "must be plan or apply")
  if (confirmed !== true) fail("confirmation", `apply requires --confirm=${PATH1_OSS_LIVE_APPLY_CONFIRMATION}`)
  validateSurfaceManifest(source)
  const store = journalStore ?? createMemoryJournalStore(source.journal ?? null)
  const journalPath = source.journalPath ?? initialContext.journalPath ?? null
  let journal = await store.load()
  if (journal) assertJournal(journal, initialContext, sourceHead, journalPath)
  else {
    journal = createJournal({ input: source, context: initialContext, sourceHead, journalPath, now: now() })
    await store.save(journal)
  }
  if (journal.status === "complete" && journal.receipt) return Object.freeze({ status: "passed", plan, receipt: journal.receipt, journal, resumed: true })
  const effectiveRunner = runner ?? createRealRunner({ cwd: initialContext.repoRoot })
  let context = initialContext
  let cells = {}
  let resumed = false
  try {
    const transfer = await executeAction({ input: source, context, sourceHead, action: "context-transfer", surfaces, runner: effectiveRunner, journal, store, now })
    journal = transfer.journal
    if (transfer.failure) return { status: journal.status, plan, failure: transfer.failure, journal }
    resumed ||= transfer.resumed
    context = resolveContextFromTransfer(context, transfer.result)
    const ready = await executeAction({ input: { ...source, sessionId: context.sessionId, roomId: context.roomId }, context, sourceHead, action: "project-environment-ready", surfaces, runner: effectiveRunner, journal, store, now })
    journal = ready.journal
    if (ready.failure) return { status: journal.status, plan, failure: ready.failure, journal }
    resumed ||= ready.resumed
    for (const provider of PATH1_REQUIRED_PROVIDERS) {
      const cell = await executeAction({ input: { ...source, sessionId: context.sessionId, roomId: context.roomId }, context, sourceHead, action: "provider-cell", provider, surfaces, runner: effectiveRunner, journal, store, now })
      journal = cell.journal
      if (cell.failure) return { status: journal.status, plan, failure: cell.failure, journal }
      resumed ||= cell.resumed
      cells[provider] = cell.result
    }
    const capabilities = await executeAction({
      input: { ...source, sessionId: context.sessionId, roomId: context.roomId, providerThreads: Object.fromEntries(PATH1_REQUIRED_PROVIDERS.map((provider) => [provider, cells[provider].threadId])) },
      context,
      sourceHead,
      action: "capability-drills",
      surfaces,
      runner: effectiveRunner,
      journal,
      store,
      now,
    })
    journal = capabilities.journal
    if (capabilities.failure) return { status: journal.status, plan, failure: capabilities.failure, journal }
    resumed ||= capabilities.resumed
    const receiptResult = {
      schema: PATH1_OSS_LIVE_RECEIPT_SCHEMA,
      status: "passed",
      mode: "live",
      source: PATH1_OSS_LIVE_SOURCE,
      liveObserved: true,
      dryRun: false,
      sourceTestOnly: false,
      observedAt: nowTimestamp(now()),
      runId: context.runId,
      sourceHead,
      allocationId: context.allocationId,
      machineId: context.machineId,
      sessionId: context.sessionId,
      roomId: context.roomId,
      providers: cells,
      capabilities: capabilities.result,
      journalRef: journal.journal.ref,
    }
    assertSanitized(receiptResult, "liveReceipt")
    if (cleanup && typeof effectiveRunner.cleanupOwned === "function") await effectiveRunner.cleanupOwned({ scope: "drill-owned", processes: journal.owned.processes, artifacts: journal.owned.artifacts })
    journal = updateJournal(journal, { status: "complete", receipt: receiptResult }, "receipt-emitted", now())
    await store.save(journal)
    return Object.freeze({ status: "passed", plan, receipt: receiptResult, journal, resumed })
  } catch (error) {
    const failure = safeFailure(error)
    journal = updateJournal(journal, { status: error?.code === "interrupted" ? "interrupted" : "failed", failure }, "run-failed", now())
    await store.save(journal)
    return { status: journal.status, plan, failure, journal }
  }
}

export async function runPath1OssLiveAction({ input, mode = "apply", confirmed = false, runner = null, journalStore = null, sourceInspector = null, now = () => new Date() } = {}) {
  const source = requireRecord(input, "input")
  if (mode === "plan") return buildPath1OssLivePlan({ input: source, sourceInspector })
  if (mode !== "apply") fail("mode", "must be plan or apply")
  if (confirmed !== true) fail("confirmation", `apply requires --confirm=${PATH1_OSS_LIVE_APPLY_CONFIRMATION}`)
  const action = source.action
  if (!["context-transfer", "project-environment-ready", "provider-cell", "capability-drills"].includes(action)) fail("action", "is unsupported")
  const provider = action === "provider-cell" ? source.provider : null
  if (provider && !PATH1_REQUIRED_PROVIDERS.includes(provider)) fail("provider", "is unsupported")
  const { receipt } = publicationReceiptFrom(source)
  const sourceHead = sourceHeadFromPublicationReceipt(receipt)
  validateContract(source, sourceHead)
  const context = resolveRunnerContext(source, sourceHead)
  await inspectSource(context.repoRoot, sourceHead, sourceInspector)
  const surfaces = await existingOfficialSurfaces(context.repoRoot)
  validateSurfaceManifest(source)
  const store = journalStore ?? createMemoryJournalStore(source.journal ?? null)
  const journalPath = source.journalPath ?? context.journalPath ?? null
  let journal = await store.load()
  if (journal) assertJournal(journal, context, sourceHead, journalPath)
  else {
    journal = createJournal({ input: source, context, sourceHead, journalPath, now: now() })
    await store.save(journal)
  }
  if (journal.status === "complete" && journal.results[stageFor(action, provider)]) {
    return Object.freeze({ ...journal.results[stageFor(action, provider)], resumed: true, journal })
  }
  let result
  try {
    result = await executeAction({ input: source, context, sourceHead, action, provider, surfaces, runner: runner ?? createRealRunner({ cwd: context.repoRoot }), journal, store, now })
  } catch (error) {
    const failure = safeFailure(error)
    const failed = updateJournal(journal, {
      status: error?.code === "interrupted" ? "interrupted" : "failed",
      failure,
    }, "action-failed", now())
    await store.save(failed)
    return { status: failed.status, failure, journal: failed }
  }
  if (result.failure) return { status: result.journal.status, failure: result.failure, journal: result.journal }
  return Object.freeze({ ...result.result, journal: result.journal, resumed: result.resumed })
}

async function readStdin() {
  const chunks = []
  let size = 0
  for await (const chunk of process.stdin) {
    size += chunk.length
    if (size > MAX_INPUT_BYTES) fail("stdin", "request is too large")
    chunks.push(chunk)
  }
  try { return JSON.parse(Buffer.concat(chunks).toString("utf8")) } catch { fail("stdin", "must contain one JSON request") }
}

function parseOptions(argv) {
  const options = {}
  for (const arg of argv) {
    const match = /^--([a-z-]+)=(.*)$/u.exec(arg)
    if (!match || Object.hasOwn(options, match[1])) fail("arguments", "must use unique --name=value options")
    options[match[1]] = match[2]
  }
  if (options.help !== undefined) return { help: true }
  if (options.mode !== undefined && !["plan", "apply"].includes(options.mode)) fail("mode", "must be plan or apply")
  return options
}

export async function main(argv = process.argv.slice(2), env = process.env, io = process) {
  const options = parseOptions(argv)
  if (options.help) {
    io.stdout.write("Usage: node scripts/path1-oss-live-driver.mjs --mode=plan|apply [--confirm=APPLY_PATH1_OSS_LIVE] [--journal=/absolute/path]\n")
    return 0
  }
  const input = await readStdin()
  const mode = options.mode ?? input.mode ?? "apply"
  const confirmed = options.confirm === PATH1_OSS_LIVE_APPLY_CONFIRMATION
    || input.confirmation === PATH1_OSS_LIVE_APPLY_CONFIRMATION
  if (options["repo-root"]) input.repoRoot = options["repo-root"]
  if (options.journal) input.journalPath = options.journal
  if (mode === "plan") {
    const result = input.action && input.action !== "run"
      ? await runPath1OssLiveAction({ input, mode, sourceInspector: null })
      : await runPath1OssLiveAcceptance({ input, mode, sourceInspector: null })
    io.stdout.write(`${JSON.stringify(result)}\n`)
    return 0
  }
  if (!confirmed) fail("confirmation", `apply requires --confirm=${PATH1_OSS_LIVE_APPLY_CONFIRMATION}`)
  const journalPath = input.journalPath ?? input.runnerContext?.journalPath
  if (!journalPath) fail("journalPath", "an external durable journal path is required for apply")
  const sourceRoot = input.repoRoot ?? input.runnerContext?.repoRoot ?? process.cwd()
  const store = createFileJournalStore(journalPath)
  const result = input.action && input.action !== "run"
    ? await runPath1OssLiveAction({ input: { ...input, repoRoot: sourceRoot }, mode, confirmed, journalStore: store })
    : await runPath1OssLiveAcceptance({ input: { ...input, repoRoot: sourceRoot }, mode, confirmed, journalStore: store })
  io.stdout.write(`${JSON.stringify(result)}\n`)
  return result.status === "passed" || result.status === "completed" ? 0 : 2
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  try {
    process.exitCode = await main()
  } catch (error) {
    process.stderr.write(`Path 1 OSS live driver blocked: ${error instanceof Path1OssLiveDriverError ? `${error.code}:${error.field}` : "invalid guarded invocation"}\n`)
    process.exitCode = 2
  }
}
