#!/usr/bin/env node

import { spawn } from "node:child_process"
import { pathToFileURL } from "node:url"
import path from "node:path"

export const PATH1_RUNNER_CONTEXT_SCHEMA = "chariox.path1.runner-context.v1"
export const PATH1_RUNNER_CONTEXT_RESULT_SCHEMA = "chariox.path1.runner-context-result.v1"
export const PATH1_RUNNER_CONTEXT_ENV = "CHARIOX_PATH1_RUNNER_CONTEXT_JSON"
export const PATH1_RUNNER_CONTEXT_CONFIRMATION = "APPLY_PATH1_OSS_LIVE"
export const PATH1_RUNNER_CONTEXT_PROVIDERS = Object.freeze(["codex", "opencode", "claude"])
export const PATH1_RUNNER_CONTEXT_CLIENT_MODES = Object.freeze(["web", "localTui", "remoteTui"])

const REVISION = /^[0-9a-f]{40}$/u
const DIGEST = /^sha256:[a-f0-9]{64}$/u
const ID = /^[A-Za-z0-9][A-Za-z0-9._:-]{2,127}$/u
const RUN_ID = /^[a-z][a-z0-9._:-]{2,80}$/u
const ABSOLUTE_PATH = /^\//u
const SECRET_KEY = /(?:token|secret|password|passwd|api[_-]?key|private[_-]?key|authorization|cookie|credential|bearer)/iu
const SECRET_VALUE = /(?:bearer\s+\S+|-----begin [^-]*private key-----|(?:sk|ghp|github_pat|xox[baprs]|akia)[a-z0-9_-]{8,})/iu
const BROAD_CLEANUP = /(?:docker\s+(?:system|container|volume|builder)\s+prune|\b(?:system|container|volume|builder)\s+prune\b|rm\s+-rf\s+(?:\/|~|\.))/iu

const OFFICIAL_SURFACES = Object.freeze({
  providerParity: "apps/cli/scripts/live-external-provider-live-parity-drill.mjs",
  nativeTuiMatrix: "apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
  nativeCommand: "apps/cli/scripts/live-native-tui-provider-command-drill.mjs",
  nativePermission: "apps/cli/scripts/live-native-tui-permission-drill.mjs",
  remoteNativeTui: "apps/cli/scripts/live-remote-native-tui-drill.mjs",
  browserComputer: "apps/cli/scripts/live-room-environment-pointer-click-drill.mjs",
  takeoverReconnect: "apps/cli/scripts/live-room-takeover-reconnect-fault-drill.mjs",
  faultRuntime: "apps/cli/scripts/lib/local-rust-fault-drill-runtime.mjs",
})

export class Path1RunnerContextAdapterError extends Error {
  constructor(field, reason, code = "missing-product-seam") {
    super(`Path 1 runner-context adapter blocked at ${field}: ${reason}`)
    this.name = "Path1RunnerContextAdapterError"
    this.field = field
    this.reason = reason
    this.code = code
  }
}

function fail(field, reason, code = "missing-product-seam") {
  throw new Path1RunnerContextAdapterError(field, reason, code)
}

function record(value, field) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(field, "must be an object")
  return value
}

function text(value, field) {
  if (typeof value !== "string" || value.trim() === "") fail(field, "must be non-empty text")
  return value
}

function boundedId(value, field) {
  if (typeof value !== "string" || !ID.test(value)) fail(field, "must be a bounded identifier")
  return value
}

function revision(value, field) {
  if (typeof value !== "string" || !REVISION.test(value)) fail(field, "must be a lowercase source revision")
  return value
}

function digest(value, field) {
  if (typeof value !== "string" || !DIGEST.test(value)) fail(field, "must be an immutable sha256 digest")
  return value
}

function absolute(value, field) {
  if (typeof value !== "string" || !ABSOLUTE_PATH.test(value)) fail(field, "must be an absolute path")
  return path.resolve(value)
}

function clone(value) {
  return JSON.parse(JSON.stringify(value))
}

function assertSafe(value, field = "value", key = "", seen = new Set()) {
  if (typeof value === "string") {
    if (SECRET_KEY.test(key) || SECRET_VALUE.test(value)) fail(field, "contains credential material", "secret-input")
    if (BROAD_CLEANUP.test(value)) fail(field, "contains broad cleanup", "unsafe-input")
    if (value.length > 200_000) fail(field, "contains oversized text", "unsafe-input")
    return value
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return value
  if (typeof value !== "object") fail(field, "contains an unsupported value", "unsafe-input")
  if (seen.has(value)) fail(field, "contains a cycle", "unsafe-input")
  seen.add(value)
  if (Array.isArray(value)) value.forEach((child, index) => assertSafe(child, `${field}[${index}]`, key, seen))
  else for (const [childKey, child] of Object.entries(value)) assertSafe(child, `${field}.${childKey}`, childKey, seen)
  seen.delete(value)
  return value
}

function endpoint(value, field, protocols) {
  const raw = text(value, field)
  let parsed
  try { parsed = new URL(raw) } catch { fail(field, "must be a URL") }
  if (!protocols.includes(parsed.protocol)) fail(field, `must use ${protocols.join(" or ")}`)
  if (parsed.username || parsed.password || parsed.search || parsed.hash) fail(field, "must not contain inline credentials or query material", "secret-input")
  return raw
}

function requireBinding(value, field, context) {
  const binding = record(value, field)
  if (binding.runId !== context.runId || binding.sessionId !== context.sessionId || binding.roomId !== context.roomId) {
    fail(field, "does not bind to the supplied Room/session")
  }
  if (binding.authority !== "home-kernel") fail(`${field}.authority`, "must be home-kernel")
  return binding
}

function receiptBindings(input, sourceHead) {
  const receipt = record(input.publicationReceipt, "publicationReceipt")
  if (receipt.schema !== "chariox.path1.oss-publication-receipt.v1" || receipt.status !== "published") {
    fail("publicationReceipt", "must be the exact reviewed published OSS receipt")
  }
  const source = record(receipt.source, "publicationReceipt.source")
  if (source.ossRevision !== sourceHead || source.treeClean !== true) fail("publicationReceipt.source", "does not bind the clean OSS head")
  const protocols = record(receipt.protocols, "publicationReceipt.protocols")
  if (protocols.localDaemon !== 333 || protocols.relayPeer !== 55) fail("publicationReceipt.protocols", "are not the reviewed exact pair")
  const image = record(receipt.image, "publicationReceipt.image")
  const runtimeDigest = digest(image.digest, "publicationReceipt.image.digest")
  if (image.signatureVerification !== "verified" || image.attestationVerification !== "verified") {
    fail("publicationReceipt.image", "signature and attestation must be verified")
  }
  const manifests = Array.isArray(image.manifests) ? image.manifests : []
  if (manifests.length !== 1 || manifests[0].platform !== "linux/amd64" || manifests[0].digest !== runtimeDigest) {
    fail("publicationReceipt.image.manifests", "must contain one immutable linux/amd64 manifest")
  }
  return { receipt, cloudRevision: revision(source.cloudRevision, "publicationReceipt.source.cloudRevision"), runtimeDigest }
}

function runtimeBindings(input, context, sourceHead, receipt) {
  const bindings = record(context.runtimeBindings ?? input.runtimeBindings, "runtimeBindings")
  if (bindings.ossRevision !== sourceHead || (context.ossRevision !== undefined && bindings.ossRevision !== context.ossRevision)) fail("runtimeBindings.ossRevision", "is stale or mismatched")
  if (bindings.cloudRevision !== receipt.cloudRevision) fail("runtimeBindings.cloudRevision", "does not match the publication receipt")
  if (bindings.runtimeDigest !== receipt.runtimeDigest) fail("runtimeBindings.runtimeDigest", "does not match the signed image digest")
  if (bindings.localDaemonProtocol !== 333 || bindings.relayPeerProtocol !== 55) fail("runtimeBindings.protocols", "are mismatched")
  return {
    cloudRevision: receipt.cloudRevision,
    ossRevision: sourceHead,
    runtimeDigest: receipt.runtimeDigest,
    localDaemonProtocol: 333,
    relayPeerProtocol: 55,
  }
}

export function validateRunnerContextRequest(input, { requireRoom = true } = {}) {
  const request = record(input, "request")
  assertSafe(request, "request")
  const context = record(request.runnerContext, "runnerContext")
  if (context.schema !== PATH1_RUNNER_CONTEXT_SCHEMA) fail("runnerContext.schema", "is unsupported")
  const runId = typeof request.runId === "string" ? request.runId : context.runId
  if (typeof runId !== "string" || !RUN_ID.test(runId)) fail("runId", "must be a lowercase bounded id")
  if (context.runId !== runId) fail("runnerContext.runId", "does not match the request")
  const sourceHead = revision(request.sourceHead ?? context.sourceHead, "sourceHead")
  const receipt = receiptBindings(request, sourceHead)
  const runtime = runtimeBindings(request, context, sourceHead, receipt)
  const contract = record(request.contract, "contract")
  if (contract.ossRevision !== sourceHead || contract.localDaemonProtocolVersion !== 333 || contract.relayPeerProtocolVersion !== 55) {
    fail("contract", "does not bind to the exact OSS/protocol contract")
  }
  const normalized = {
    schema: PATH1_RUNNER_CONTEXT_SCHEMA,
    runId,
    allocationId: boundedId(context.allocationId, "runnerContext.allocationId"),
    machineId: boundedId(context.machineId, "runnerContext.machineId"),
    homeKernelId: boundedId(context.homeKernelId, "runnerContext.homeKernelId"),
    homeRelayRealmId: boundedId(context.homeRelayRealmId, "runnerContext.homeRelayRealmId"),
    repoRoot: absolute(context.repoRoot, "runnerContext.repoRoot"),
    sourceHead,
    sessionId: boundedId(request.sessionId ?? context.sessionId, "runnerContext.sessionId"),
    roomId: boundedId(request.roomId ?? context.roomId, "runnerContext.roomId"),
    kernelEndpoint: endpoint(context.kernelEndpoint ?? context.kernelUrl, "runnerContext.kernelEndpoint", ["ws:", "wss:"]),
    relayEndpoint: endpoint(context.relayEndpoint ?? context.relayUrl, "runnerContext.relayEndpoint", ["ws:", "wss:"]),
    webEndpoint: endpoint(context.webEndpoint, "runnerContext.webEndpoint", ["http:", "https:"]),
    journal: record(context.journal, "runnerContext.journal"),
    runtimeBindings: runtime,
    surfaceManifest: record(context.surfaceManifest, "runnerContext.surfaceManifest"),
    clientModes: record(context.clientModes, "runnerContext.clientModes"),
    clientBindings: record(context.clientBindings, "runnerContext.clientBindings"),
    authority: record(context.authority, "runnerContext.authority"),
    officialSurfaces: record(context.officialSurfaces ?? {}, "runnerContext.officialSurfaces"),
  }
  if (context.evidenceRoot !== undefined) normalized.evidenceRoot = absolute(context.evidenceRoot, "runnerContext.evidenceRoot")
  if (!Number.isSafeInteger(normalized.journal.sequence) || normalized.journal.sequence < 0) fail("runnerContext.journal.sequence", "must be a monotonic journal sequence")
  text(normalized.journal.ref, "runnerContext.journal.ref")
  if (normalized.journal.path !== undefined) absolute(normalized.journal.path, "runnerContext.journal.path")
  for (const field of ["sameRoom", "acceptsRunnerContext", "normalKernelAuthority", "relayTransportOnly"]) {
    if (normalized.surfaceManifest[field] !== true) fail(`runnerContext.surfaceManifest.${field}`, "must be true")
  }
  for (const mode of PATH1_RUNNER_CONTEXT_CLIENT_MODES) {
    if (normalized.clientModes[mode] !== true) fail(`runnerContext.clientModes.${mode}`, "is required")
    requireBinding(normalized.clientBindings[mode], `runnerContext.clientBindings.${mode}`, normalized)
  }
  if (normalized.authority.kernel !== "normal-kernel" || normalized.authority.relay !== "transport-only" || normalized.authority.room !== "kernel-owned") {
    fail("runnerContext.authority", "must preserve home-kernel Room authority")
  }
  if (requireRoom && (!normalized.sessionId || !normalized.roomId)) fail("runnerContext", "an existing session and Room are required")
  const provider = request.provider ?? null
  if (provider !== null && !PATH1_RUNNER_CONTEXT_PROVIDERS.includes(provider)) fail("provider", "is unsupported")
  const action = text(request.action, "action")
  if (!["context-transfer", "project-environment-ready", "provider-cell", "capability-drills"].includes(action)) fail("action", "is unsupported")
  return Object.freeze({ ...normalized, action, provider, receipt, sourceHead })
}

export function parseOptionalRunnerContext(value) {
  if (value === undefined || value === null || value === "") return null
  if (typeof value === "object") {
    assertSafe(value, "runnerContext")
    return clone(value)
  }
  try {
    const parsed = JSON.parse(String(value))
    assertSafe(parsed, "runnerContext")
    return parsed
  } catch (error) {
    if (error instanceof Path1RunnerContextAdapterError) throw error
    fail("runnerContext", "must contain valid JSON", "invalid-input")
  }
}

export function validateExistingRoomContext(input, { expected = {} } = {}) {
  const value = record(input, "runnerContext")
  assertSafe(value, "runnerContext")
  if (value.schema !== PATH1_RUNNER_CONTEXT_SCHEMA) fail("runnerContext.schema", "is unsupported", "stale-context")
  const normalized = {
    schema: PATH1_RUNNER_CONTEXT_SCHEMA,
    runId: value.runId,
    allocationId: value.allocationId,
    machineId: value.machineId,
    homeKernelId: value.homeKernelId,
    homeRelayRealmId: value.homeRelayRealmId,
    repoRoot: absolute(value.repoRoot, "runnerContext.repoRoot"),
    sourceHead: revision(value.sourceHead, "runnerContext.sourceHead"),
    sessionId: boundedId(value.sessionId, "runnerContext.sessionId"),
    roomId: boundedId(value.roomId, "runnerContext.roomId"),
    kernelEndpoint: endpoint(value.kernelEndpoint ?? value.kernelUrl, "runnerContext.kernelEndpoint", ["ws:", "wss:"]),
    relayEndpoint: endpoint(value.relayEndpoint ?? value.relayUrl, "runnerContext.relayEndpoint", ["ws:", "wss:"]),
    webEndpoint: endpoint(value.webEndpoint, "runnerContext.webEndpoint", ["http:", "https:"]),
    journal: record(value.journal, "runnerContext.journal"),
    runtimeBindings: record(value.runtimeBindings, "runnerContext.runtimeBindings"),
    surfaceManifest: record(value.surfaceManifest, "runnerContext.surfaceManifest"),
    clientModes: record(value.clientModes, "runnerContext.clientModes"),
    clientBindings: record(value.clientBindings, "runnerContext.clientBindings"),
    authority: record(value.authority, "runnerContext.authority"),
    officialSurfaces: record(value.officialSurfaces ?? {}, "runnerContext.officialSurfaces"),
    action: value.action,
    provider: value.provider ?? null,
  }
  if (!RUN_ID.test(normalized.runId ?? "")) fail("runnerContext.runId", "is stale or malformed", "stale-context")
  boundedId(normalized.allocationId, "runnerContext.allocationId")
  boundedId(normalized.machineId, "runnerContext.machineId")
  boundedId(normalized.homeKernelId, "runnerContext.homeKernelId")
  boundedId(normalized.homeRelayRealmId, "runnerContext.homeRelayRealmId")
  if (normalized.provider !== null && !PATH1_RUNNER_CONTEXT_PROVIDERS.includes(normalized.provider)) fail("runnerContext.provider", "is unsupported", "stale-context")
  if (normalized.action !== undefined && !["context-transfer", "project-environment-ready", "provider-cell", "capability-drills"].includes(normalized.action)) {
    fail("runnerContext.action", "is unsupported", "stale-context")
  }
  if (value.evidenceRoot !== undefined) normalized.evidenceRoot = absolute(value.evidenceRoot, "runnerContext.evidenceRoot")
  if (!Number.isSafeInteger(normalized.journal.sequence) || normalized.journal.sequence < 0) {
    fail("runnerContext.journal.sequence", "must be a monotonic journal sequence", "stale-context")
  }
  text(normalized.journal.ref, "runnerContext.journal.ref")
  if (normalized.journal.path !== undefined) absolute(normalized.journal.path, "runnerContext.journal.path")
  const bindings = normalized.runtimeBindings
  if (bindings.ossRevision !== normalized.sourceHead) fail("runnerContext.runtimeBindings.ossRevision", "is stale", "stale-context")
  revision(bindings.cloudRevision, "runnerContext.runtimeBindings.cloudRevision")
  digest(bindings.runtimeDigest, "runnerContext.runtimeBindings.runtimeDigest")
  if (bindings.localDaemonProtocol !== 333 || bindings.relayPeerProtocol !== 55) {
    fail("runnerContext.runtimeBindings.protocols", "are mismatched", "stale-context")
  }
  if (value.receipt !== undefined) {
    const receipt = record(value.receipt, "runnerContext.receipt")
    const image = record(receipt.image, "runnerContext.receipt.image")
    if (image.digest !== bindings.runtimeDigest || receipt.status !== "published") {
      fail("runnerContext.receipt", "is stale or mismatched", "stale-context")
    }
  }
  for (const field of ["sameRoom", "acceptsRunnerContext", "normalKernelAuthority", "relayTransportOnly"]) {
    if (normalized.surfaceManifest[field] !== true) fail(`runnerContext.surfaceManifest.${field}`, "must be true", "stale-context")
  }
  for (const mode of PATH1_RUNNER_CONTEXT_CLIENT_MODES) {
    if (normalized.clientModes[mode] !== true) fail(`runnerContext.clientModes.${mode}`, "is required", "stale-context")
    requireBinding(normalized.clientBindings[mode], `runnerContext.clientBindings.${mode}`, normalized)
  }
  if (normalized.authority.kernel !== "normal-kernel" || normalized.authority.relay !== "transport-only" || normalized.authority.room !== "kernel-owned") {
    fail("runnerContext.authority", "must preserve home-kernel Room authority", "stale-context")
  }
  for (const [field, expectedValue] of Object.entries(expected)) {
    if (expectedValue !== undefined && normalized[field] !== expectedValue) fail(`runnerContext.${field}`, "does not match the supplied context", "stale-context")
  }
  return Object.freeze({ ...value, ...normalized })
}

export function buildStrictChildEnvironment(context, baseEnvironment = {}) {
  const normalized = record(context, "runnerContext")
  const allowed = ["PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR"]
  const environment = {}
  for (const name of allowed) if (typeof baseEnvironment[name] === "string") environment[name] = baseEnvironment[name]
  environment.CHARIOX_PATH1_RUNNER_CONTEXT_JSON = JSON.stringify(normalized)
  environment.CHARIOX_PATH1_RUN_ID = normalized.runId
  environment.CHARIOX_PATH1_SOURCE_HEAD = normalized.sourceHead
  environment.CHARIOX_PATH1_RUNTIME_DIGEST = normalized.runtimeBindings.runtimeDigest
  environment.CHARIOX_PATH1_JOURNAL_REF = normalized.journal.ref
  environment.CHARIOX_PATH1_CONTEXT_SCHEMA = PATH1_RUNNER_CONTEXT_SCHEMA
  environment.CHARIOX_PATH1_CONTEXT_MODE = "required-existing-room"
  environment.CHARIOX_PATH1_ACTION = normalized.action
  return Object.freeze(environment)
}

function surfacePath(context, key) {
  const relative = OFFICIAL_SURFACES[key]
  const supplied = context.officialSurfaces[key]
  const absolutePath = supplied ?? path.join(context.repoRoot, relative)
  if (!absolutePath.startsWith(`${context.repoRoot}/`) && absolutePath !== context.repoRoot) fail(`officialSurfaces.${key}`, "must remain inside the exact source root")
  return absolutePath
}

export function buildExistingRoomCellInvocation({ context, surface, provider = null, extraArgs = [], role = "official-cell" } = {}) {
  const normalized = validateExistingRoomContext(context, { expected: provider ? { provider } : {} })
  const args = [surfacePath(normalized, surface), "--path1-runner-context", ...extraArgs]
  if (provider && !extraArgs.includes("--providers")) args.push("--providers", provider)
  return Object.freeze({
    id: `${role}:${provider ?? "room"}`,
    role,
    surface,
    command: process.execPath,
    args,
    cwd: normalized.repoRoot,
    env: buildStrictChildEnvironment(normalized, process.env),
    runId: normalized.runId,
    sessionId: normalized.sessionId,
    roomId: normalized.roomId,
    provider,
    createKernel: false,
    createSession: false,
    createRelay: false,
    sameRoomRequired: true,
  })
}

export function buildOfficialInvocationPlan(context) {
  const env = buildStrictChildEnvironment(context, process.env)
  const invocation = (id, surface, extraArgs = [], flags = {}) => ({
    id,
    surface,
    command: process.execPath,
    args: [surfacePath(context, surface), "--path1-runner-context", ...extraArgs],
    env,
    browserFirst: flags.browserFirst === true,
    computerFallback: flags.computerFallback === true,
    provider: flags.provider ?? null,
  })
  if (context.action === "context-transfer") return Object.freeze([])
  if (context.action === "project-environment-ready") {
    return Object.freeze([invocation("project-environment-ready", "browserComputer", [], { browserFirst: true })])
  }
  if (context.action === "provider-cell") {
    const nativeSurface = context.provider === "claude" ? "nativePermission" : "nativeCommand"
    return Object.freeze([
      invocation(`provider:${context.provider}:browser`, "providerParity", ["--providers", context.provider], { browserFirst: true, provider: context.provider }),
      invocation(`provider:${context.provider}:computer-fallback`, nativeSurface, ["--providers", context.provider], { computerFallback: true, provider: context.provider }),
    ])
  }
  return Object.freeze([
    invocation("capability:browser", "browserComputer", [], { browserFirst: true }),
    invocation("capability:computer-fallback", "remoteNativeTui", [], { computerFallback: true }),
    invocation("capability:takeover-reconnect", "faultRuntime"),
  ])
}

function safeResult(value, field) {
  const result = record(value, field)
  assertSafe(result, field)
  if (result.liveObserved !== true || result.dryRun === true || result.sourceTestOnly === true) fail(field, "must be live observed evidence")
  if (result.runId === undefined || result.sessionId === undefined || result.roomId === undefined) fail(field, "must include run/session/Room binding")
  return result
}

function verifyResultBinding(result, context, field) {
  const safe = safeResult(result, field)
  if (safe.runId !== context.runId || safe.sessionId !== context.sessionId || safe.roomId !== context.roomId) fail(field, "is stale or bound to a different Room")
  if (safe.sourceHead !== undefined && safe.sourceHead !== context.sourceHead) fail(`${field}.sourceHead`, "is stale")
  return safe
}

function receiptId(value, field) {
  return boundedId(value, field)
}

function journalEvent(journalWriter, event) {
  if (!journalWriter) return
  if (typeof journalWriter.append !== "function") fail("journalWriter", "must expose append only")
  assertSafe(event, "journalEvent")
  return journalWriter.append(clone(event))
}

async function journalResults(journalWriter) {
  if (!journalWriter || typeof journalWriter.load !== "function") return {}
  const state = await journalWriter.load()
  if (!state || typeof state !== "object" || Array.isArray(state)) return {}
  return state.results && typeof state.results === "object" && !Array.isArray(state.results) ? state.results : {}
}

async function attachClients(context, clientFactory) {
  const clients = {}
  for (const mode of PATH1_RUNNER_CONTEXT_CLIENT_MODES) {
    if (!clientFactory || typeof clientFactory.attach !== "function") fail("clientFactory", "cannot attach all required clients", "missing-product-seam")
    const attached = await clientFactory.attach({
      mode,
      endpoint: mode === "remoteTui" ? context.relayEndpoint : mode === "web" ? context.webEndpoint : context.kernelEndpoint,
      runId: context.runId,
      sessionId: context.sessionId,
      roomId: context.roomId,
      authority: "home-kernel",
    })
    const binding = record(attached?.binding ?? attached, `client.${mode}`)
    if (binding.runId !== context.runId || binding.sessionId !== context.sessionId || binding.roomId !== context.roomId) fail(`client.${mode}`, "attached to a different Room")
    if (binding.authority !== "home-kernel") fail(`client.${mode}.authority`, "must be home-kernel")
    clients[mode] = attached
  }
  return clients
}

async function handshakeContext(context, clientFactory) {
  const clients = await attachClients(context, clientFactory)
  if (typeof clientFactory.verifyRoom !== "function") fail("clientFactory.verifyRoom", "must verify the supplied Room binding")
  const observed = await clientFactory.verifyRoom({
    clients,
    runId: context.runId,
    sessionId: context.sessionId,
    roomId: context.roomId,
    homeKernelId: context.homeKernelId,
  })
  const binding = record(observed, "clientFactory.verifyRoom")
  if (binding.runId !== context.runId || binding.sessionId !== context.sessionId || binding.roomId !== context.roomId || binding.kernelAuthoritative !== true || binding.relayTransportOnly !== true) {
    fail("clientFactory.verifyRoom", "did not prove the same home-kernel Room")
  }
  return { clients, binding }
}

function requireOfficialEvidence(value, context, field) {
  const evidence = record(value?.path1Result ?? value?.runnerContextResult ?? value, field)
  const result = verifyResultBinding(evidence, context, field)
  if (result.source !== "deployed-oss-live-drill") fail(`${field}.source`, "must be the deployed OSS live surface")
  if (result.official !== true || result.kernelAuthoritative !== true || result.relayTransportOnly !== true) fail(field, "must be official kernel-authoritative evidence")
  receiptId(result.receiptId, `${field}.receiptId`)
  return result
}

function combineProviderEvidence(outputs, context) {
  const browser = requireOfficialEvidence(outputs[0], context, "provider.browser")
  const computer = requireOfficialEvidence(outputs[1], context, "provider.computerFallback")
  if (browser.provider !== context.provider || computer.provider !== context.provider) fail("provider", "official outputs are for a different provider")
  if (!browser.browser && !browser.browserEvidence) fail("provider.browser", "Browser evidence is required")
  if (!computer.computerFallback && !computer.computerEvidence) fail("provider.computerFallback", "Computer fallback evidence is required")
  boundedId(browser.actorId ?? computer.actorId, "provider.actorId")
  boundedId(browser.threadId ?? computer.threadId, "provider.threadId")
  return {
    ...browser,
    schema: PATH1_RUNNER_CONTEXT_RESULT_SCHEMA,
    action: "provider-cell",
    provider: context.provider,
    official: true,
    officialHarness: context.provider,
    surfaces: [...new Set([...(browser.surfaces ?? []), ...(computer.surfaces ?? [])])],
    actorId: browser.actorId ?? computer.actorId,
    threadId: browser.threadId ?? computer.threadId,
    browser: browser.browser ?? browser.browserEvidence,
    computerFallback: computer.computerFallback ?? computer.computerEvidence,
    completion: browser.completion ?? computer.completion,
    history: browser.history ?? computer.history,
    receiptId: receiptId(computer.receiptId, "provider.computerFallback.receiptId"),
  }
}

function combineCapabilityEvidence(outputs, context) {
  const browser = requireOfficialEvidence(outputs[0], context, "capability.browser")
  const computer = requireOfficialEvidence(outputs[1], context, "capability.computerFallback")
  const takeover = requireOfficialEvidence(outputs[2], context, "capability.takeoverReconnect")
  const capability = browser.capabilities ?? browser.capabilityEvidence
  if (!capability || computer.capabilities === undefined && computer.capabilityEvidence === undefined || takeover.capabilities === undefined && takeover.capabilityEvidence === undefined) {
    fail("capabilityEvidence", "official Browser, Computer, and takeover evidence is required")
  }
  const merged = {
    ...(capability ?? {}),
    ...(computer.capabilities ?? computer.capabilityEvidence ?? {}),
    ...(takeover.capabilities ?? takeover.capabilityEvidence ?? {}),
  }
  return {
    ...merged,
    schema: PATH1_RUNNER_CONTEXT_RESULT_SCHEMA,
    action: "capability-drills",
    source: "deployed-oss-live-drill",
    liveObserved: true,
    dryRun: false,
    sourceTestOnly: false,
    runId: context.runId,
    sessionId: context.sessionId,
    roomId: context.roomId,
    sourceHead: context.sourceHead,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    official: true,
    capabilities: merged,
    receiptId: receiptId(takeover.receiptId, "capability.receiptId"),
  }
}

async function defaultClientFactory(context) {
  const ipc = await import(pathToFileURL(path.join(context.repoRoot, "packages", "kernel-client", "dist", "ipc.js")).href)
  const requests = await import(pathToFileURL(path.join(context.repoRoot, "packages", "kernel-client", "dist", "ipc-session-requests.js")).href)
  const clients = []
  return {
    async attach({ mode, endpoint, sessionId, runId }) {
      const client = new ipc.LocalIpcClient(endpoint)
      const response = await client.send(requests.attachToSessionRequest(sessionId, `${runId}-path1-${mode}`))
      const attached = response.AttachedToSession ?? response.SessionAttached ?? response
      const binding = {
        runId,
        sessionId: attached.session?.id ?? attached.session_id ?? sessionId,
        roomId: context.roomId,
        authority: "home-kernel",
      }
      clients.push(client)
      return { client, binding }
    },
    async verifyRoom({ runId, sessionId, roomId }) {
      if (clients.length !== PATH1_RUNNER_CONTEXT_CLIENT_MODES.length) fail("clientFactory.verifyRoom", "not all clients attached")
      return { runId, sessionId, roomId, kernelAuthoritative: true, relayTransportOnly: true }
    },
    async close() {
      await Promise.all(clients.map((client) => client.close?.()))
    },
  }
}

async function invokeOfficial(invocation, processRunner) {
  if (!processRunner || typeof processRunner.run !== "function") fail("processRunner", "must execute the official drill")
  return processRunner.run(invocation)
}

function defaultOfficialProcessRunner() {
  return {
    run(invocation) {
      return new Promise((resolve, reject) => {
        const child = spawn(invocation.command, invocation.args, {
          cwd: invocation.cwd,
          env: invocation.env,
          stdio: ["ignore", "pipe", "pipe"],
        })
        const stdout = []
        const stderr = []
        const timer = setTimeout(() => {
          try { child.kill("SIGTERM") } catch {}
          reject(new Path1RunnerContextAdapterError(invocation.id, "official drill timed out", "official-timeout"))
        }, 300_000)
        child.stdout.on("data", (chunk) => stdout.push(chunk))
        child.stderr.on("data", (chunk) => stderr.push(chunk))
        child.once("error", (error) => {
          clearTimeout(timer)
          void error
          reject(new Path1RunnerContextAdapterError(invocation.id, "official drill could not start", "official-process"))
        })
        child.once("close", (code, signal) => {
          clearTimeout(timer)
          const output = `${Buffer.concat(stdout).toString("utf8")}\n${Buffer.concat(stderr).toString("utf8")}`
          const marker = output.split(/\r?\n/u).find((line) => line.startsWith("CHARIOX_PATH1_RESULT:"))
          if (!marker) {
            if (code !== 0 || signal) {
              reject(new Path1RunnerContextAdapterError(invocation.id, "official drill failed; preserved runner evidence is required", "official-failed"))
              return
            }
            reject(new Path1RunnerContextAdapterError(invocation.id, "official drill did not emit a typed Path 1 result", "missing-product-seam"))
            return
          }
          try {
            const result = JSON.parse(marker.slice("CHARIOX_PATH1_RESULT:".length))
            if (code !== 0 || signal) {
              resolve(result)
              return
            }
            resolve(result)
          } catch {
            reject(new Path1RunnerContextAdapterError(invocation.id, "official drill emitted invalid typed evidence", "invalid-evidence"))
          }
        })
      })
    },
  }
}

export async function runExistingRoomCell({
  context,
  surface,
  provider = null,
  extraArgs = [],
  role = "official-cell",
  processRunner = null,
  clientFactory = null,
} = {}) {
  const normalized = validateExistingRoomContext(context, { expected: provider ? { provider } : {} })
  const invocation = buildExistingRoomCellInvocation({ context: normalized, surface, provider, extraArgs, role })
  const factory = clientFactory ?? await defaultClientFactory(normalized)
  try {
    const handshake = await handshakeContext(normalized, factory)
    if (handshake.binding.kernelAuthoritative !== true || handshake.binding.relayTransportOnly !== true) {
      fail("clientFactory.verifyRoom", "did not prove the supplied home-kernel Room")
    }
    const output = await invokeOfficial(invocation, processRunner ?? defaultOfficialProcessRunner())
    const evidence = requireOfficialEvidence(output, normalized, `${role}.result`)
    return Object.freeze({
      ...evidence,
      cellRole: role,
      sessionId: normalized.sessionId,
      roomId: normalized.roomId,
      runId: normalized.runId,
      sameRoom: true,
      noSecondAuthority: true,
    })
  } catch (error) {
    if (error instanceof Path1RunnerContextAdapterError) throw error
    fail(`${role}.process`, "official cell failed; runner-owned failure evidence is preserved", "official-failed")
  } finally {
    await factory.close?.()
  }
}

export async function runPath1RunnerContextAdapter({
  input,
  mode = "plan",
  confirmed = false,
  processRunner = null,
  clientFactory = null,
  journalWriter = null,
} = {}) {
  const context = validateRunnerContextRequest(input)
  const plan = buildOfficialInvocationPlan(context)
  const planResult = Object.freeze({
    schema: PATH1_RUNNER_CONTEXT_RESULT_SCHEMA,
    mode: "plan",
    action: context.action,
    runId: context.runId,
    sourceHead: context.sourceHead,
    sessionId: context.sessionId,
    roomId: context.roomId,
    runtimeDigest: context.runtimeBindings.runtimeDigest,
    browserFirst: true,
    sameRoomRequired: true,
    invocationOrder: plan.map((item) => item.id),
    invocations: plan.map(({ env, ...item }) => ({ ...item, envKeys: Object.keys(env).sort() })),
  })
  if (mode === "plan") return planResult
  if (mode !== "apply") fail("mode", "must be plan or apply", "invalid-input")
  if (confirmed !== true) fail("confirmation", `apply requires ${PATH1_RUNNER_CONTEXT_CONFIRMATION}`, "confirmation-required")

  const factory = clientFactory ?? await defaultClientFactory(context)
  const handshake = await handshakeContext(context, factory)
  await journalEvent(journalWriter, {
    type: "runner-context-attached",
    runId: context.runId,
    sessionId: context.sessionId,
    roomId: context.roomId,
    journalRef: context.journal.ref,
  })
  const outputs = []
  const resumedResults = await journalResults(journalWriter)
  try {
    if (context.action === "context-transfer") {
      const result = {
        schema: PATH1_RUNNER_CONTEXT_RESULT_SCHEMA,
        source: "deployed-oss-live-drill",
        liveObserved: true,
        dryRun: false,
        sourceTestOnly: false,
        action: "context-transfer",
        status: "completed",
        runId: context.runId,
        sourceHead: context.sourceHead,
        allocationId: context.allocationId,
        machineId: context.machineId,
        sessionId: context.sessionId,
        roomId: context.roomId,
        kernelAuthoritative: handshake.binding.kernelAuthoritative,
        relayTransportOnly: handshake.binding.relayTransportOnly,
        official: true,
        receiptId: `${context.runId}-context-transfer`,
      }
      assertSafe(result, "result")
      await journalEvent(journalWriter, { type: "runner-context-verified", runId: context.runId, sessionId: context.sessionId, roomId: context.roomId })
      return Object.freeze(result)
    }
    for (const invocation of plan) {
      if (resumedResults[invocation.id]) {
        outputs.push(resumedResults[invocation.id])
        continue
      }
      const output = await invokeOfficial(invocation, processRunner)
      outputs.push(output)
      await journalEvent(journalWriter, { type: "official-cell", invocationId: invocation.id, result: output })
    }
    const result = context.action === "provider-cell"
      ? combineProviderEvidence(outputs, context)
      : context.action === "capability-drills"
        ? combineCapabilityEvidence(outputs, context)
        : requireOfficialEvidence(outputs[0], context, "project-environment-ready")
    const finalResult = {
      ...result,
      source: "deployed-oss-live-drill",
      action: context.action,
      runId: context.runId,
      sourceHead: context.sourceHead,
      allocationId: context.allocationId,
      machineId: context.machineId,
      sessionId: context.sessionId,
      roomId: context.roomId,
      runtimeDigest: context.runtimeBindings.runtimeDigest,
      kernelAuthoritative: true,
      relayTransportOnly: true,
      official: true,
    }
    assertSafe(finalResult, "result")
    await journalEvent(journalWriter, { type: "runner-context-result", runId: context.runId, action: context.action, receiptId: finalResult.receiptId })
    return Object.freeze(finalResult)
  } catch (error) {
    await journalEvent(journalWriter, {
      type: "runner-context-failure",
      runId: context.runId,
      action: context.action,
      code: error?.code === "interrupted" ? "interrupted" : "adapter-failed",
      field: error?.field ?? null,
      reason: error instanceof Path1RunnerContextAdapterError ? error.reason : "official runner-context cell failed",
    })
    throw error
  } finally {
    await factory.close?.()
  }
}

async function readStdin() {
  const chunks = []
  for await (const chunk of process.stdin) chunks.push(chunk)
  try { return JSON.parse(Buffer.concat(chunks).toString("utf8")) } catch { fail("stdin", "must contain one JSON request", "invalid-input") }
}

export async function main(argv = process.argv.slice(2), io = process) {
  const mode = argv.find((arg) => arg.startsWith("--mode="))?.slice("--mode=".length) ?? "apply"
  const confirmed = argv.some((arg) => arg === `--confirm=${PATH1_RUNNER_CONTEXT_CONFIRMATION}`)
  const input = await readStdin()
  try {
    const result = await runPath1RunnerContextAdapter({ input, mode, confirmed })
    io.stdout.write(`${JSON.stringify(result)}\n`)
    return 0
  } catch (error) {
    const safe = error instanceof Path1RunnerContextAdapterError
      ? { schema: PATH1_RUNNER_CONTEXT_RESULT_SCHEMA, status: "failed", failure: { code: error.code, field: error.field, reason: error.reason } }
      : { schema: PATH1_RUNNER_CONTEXT_RESULT_SCHEMA, status: "failed", failure: { code: "adapter-failed", field: null, reason: "runner-context adapter failed" } }
    io.stdout.write(`${JSON.stringify(safe)}\n`)
    return 2
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  process.exitCode = await main()
}
