import { createHash } from "node:crypto"
import { isAbsolute } from "node:path"

export const PROJECT_SETUP_SELECTION_SCHEMA = "chariox.managed-ordinary-project-setup-selection/v1"
export const PROJECT_SETUP_PROOF_SCHEMA = "chariox.managed-ordinary-project-setup-proof/v1"
export const PROJECT_SETUP_MAX_STATUS_AGE_MS = 15 * 60 * 1000
export const PROJECT_SETUP_OBSERVATION_TIMEOUT_MS = 60 * 1000
export const PROJECT_SETUP_REQUEST_TIMEOUT_MS = 10 * 1000

export const PROJECT_SETUP_PROOF_KEYS = Object.freeze([
  "schema",
  "product_api_observation",
  "ready_validation_verified",
  "status_fresh",
  "before_after_identity_stable",
  "home_kernel_identity_fingerprint",
  "session_identity_fingerprint",
  "agent_identity_fingerprint",
  "project_identity_fingerprint",
  "operation_identity_fingerprint",
  "operation_attempt",
  "operation_created_at_ms",
  "status_updated_at_ms",
  "observation_started_at_ms",
  "observation_finished_at_ms",
  "worker_identity_fingerprint",
  "worker_kernel_identity_fingerprint",
  "target_identity_digest",
  "platform",
  "definition_digest",
  "definition_origin",
  "definition_source",
  "definition_identity_verified",
  "validation_command_count",
  "validation_receipts",
  "before_snapshot_digest",
  "after_snapshot_digest",
  "transport_kind",
  "endpoint_fingerprint",
])

const IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9_.:/-]{0,127}$/
const SHA256 = /^sha256:[0-9a-f]{64}$/i
const DEFINITION_ORIGINS = new Set(["user_authored", "utility_generated"])
const DEFINITION_SOURCES = new Set(["commands", "dockerfile", "devcontainer", "setup_script"])
const NODE_URL = new URL(import.meta.url)

export class ProjectSetupObserverError extends Error {
  constructor(code, message, options = {}) {
    super(message, options)
    this.name = "ProjectSetupObserverError"
    this.code = code
  }
}

function fail(code, message, cause) {
  throw new ProjectSetupObserverError(code, message, cause ? { cause } : undefined)
}

function fingerprint(value) {
  return `sha256:${createHash("sha256").update(String(value), "utf8").digest("hex")}`
}

function stable(value) {
  if (Array.isArray(value)) return value.map(stable)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stable(value[key])]))
}

function stableJson(value) {
  return JSON.stringify(stable(value))
}

function isPlainObject(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false
  const prototype = Object.getPrototypeOf(value)
  return prototype === Object.prototype || prototype === null
}

function exactKeys(value, expected) {
  return isPlainObject(value)
    && Object.keys(value).length === expected.length
    && expected.every((key) => Object.hasOwn(value, key))
}

export function projectSetupTargetIdentityDigest(workerIdentityFingerprint, workerKernelIdentityFingerprint, platform) {
  return fingerprint(stableJson({
    worker_identity_fingerprint: workerIdentityFingerprint,
    worker_kernel_identity_fingerprint: workerKernelIdentityFingerprint,
    platform,
  }))
}

export function validateProjectSetupProof(value) {
  const invalid = (code) => ({ ok: false, code })
  if (!exactKeys(value, PROJECT_SETUP_PROOF_KEYS)) return invalid("proof_shape_invalid")
  if (value.schema !== PROJECT_SETUP_PROOF_SCHEMA) return invalid("proof_schema_mismatch")
  if (value.product_api_observation !== "kernel-public-api") return invalid("public_api_observation_required")
  if (value.transport_kind !== "kernel-public-api" && value.transport_kind !== "relay") {
    return invalid("public_api_transport_required")
  }
  for (const field of [
    "ready_validation_verified",
    "status_fresh",
    "before_after_identity_stable",
    "definition_identity_verified",
  ]) {
    if (value[field] !== true) return invalid(`${field}_required`)
  }
  for (const field of [
    "home_kernel_identity_fingerprint",
    "session_identity_fingerprint",
    "agent_identity_fingerprint",
    "project_identity_fingerprint",
    "operation_identity_fingerprint",
    "worker_identity_fingerprint",
    "worker_kernel_identity_fingerprint",
    "target_identity_digest",
    "definition_digest",
    "before_snapshot_digest",
    "after_snapshot_digest",
    "endpoint_fingerprint",
  ]) {
    if (typeof value[field] !== "string" || !SHA256.test(value[field])) return invalid(`${field}_invalid`)
  }
  if (!Number.isSafeInteger(value.operation_attempt) || value.operation_attempt < 1) {
    return invalid("operation_attempt_invalid")
  }
  for (const field of [
    "operation_created_at_ms",
    "status_updated_at_ms",
    "observation_started_at_ms",
    "observation_finished_at_ms",
  ]) {
    if (!Number.isSafeInteger(value[field]) || value[field] < 0) return invalid(`${field}_invalid`)
  }
  if (value.operation_created_at_ms > value.status_updated_at_ms
    || value.status_updated_at_ms > value.observation_finished_at_ms
    || value.observation_started_at_ms > value.observation_finished_at_ms) {
    return invalid("proof_timestamps_out_of_order")
  }
  if (value.observation_finished_at_ms - value.status_updated_at_ms > PROJECT_SETUP_MAX_STATUS_AGE_MS) {
    return invalid("operation_status_stale")
  }
  if (value.observation_finished_at_ms - value.observation_started_at_ms > PROJECT_SETUP_OBSERVATION_TIMEOUT_MS) {
    return invalid("observation_too_long")
  }
  if (typeof value.platform !== "string" || !value.platform.trim()) return invalid("platform_invalid")
  if (!DEFINITION_ORIGINS.has(value.definition_origin)) return invalid("definition_origin_invalid")
  if (!DEFINITION_SOURCES.has(value.definition_source)) return invalid("definition_source_invalid")
  if (!Number.isSafeInteger(value.validation_command_count) || value.validation_command_count < 1) {
    return invalid("validation_command_count_invalid")
  }
  if (!Array.isArray(value.validation_receipts)
    || value.validation_receipts.length !== value.validation_command_count
    || value.validation_receipts.length === 0) {
    return invalid("validation_receipts_missing")
  }
  for (const receipt of value.validation_receipts) {
    if (!exactKeys(receipt, ["command_digest", "exit_code", "stdout_bytes", "stderr_bytes"])) {
      return invalid("validation_receipt_shape_invalid")
    }
    if (typeof receipt.command_digest !== "string" || !SHA256.test(receipt.command_digest)) {
      return invalid("validation_receipt_digest_invalid")
    }
    if (receipt.exit_code !== 0
      || !Number.isSafeInteger(receipt.stdout_bytes) || receipt.stdout_bytes < 0
      || !Number.isSafeInteger(receipt.stderr_bytes) || receipt.stderr_bytes < 0) {
      return invalid("validation_receipt_failed")
    }
  }
  if (value.before_snapshot_digest !== value.after_snapshot_digest) return invalid("before_after_identity_mismatch")
  if (value.target_identity_digest !== projectSetupTargetIdentityDigest(
    value.worker_identity_fingerprint,
    value.worker_kernel_identity_fingerprint,
    value.platform,
  )) return invalid("target_identity_digest_mismatch")
  return { ok: true, code: null }
}

export function assertProjectSetupProof(value) {
  const validation = validateProjectSetupProof(value)
  if (!validation.ok) fail(validation.code, `Project setup proof failed validation: ${validation.code}`)
  return value
}

function validIdentifier(value) {
  return typeof value === "string" && IDENTIFIER.test(value)
}

function requiredIdentifier(value, label) {
  if (!validIdentifier(value)) fail("selection_invalid", `Project setup selection is missing a valid ${label}`)
  return value
}

function requiredTime(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) fail("selection_invalid", `Project setup selection is missing a valid ${label}`)
  return value
}

function parseEndpoint(value) {
  if (typeof value !== "string" || !value.trim()) {
    fail("selection_invalid", "Project setup selection is missing a kernel endpoint")
  }
  const endpoint = value.trim()
  if (isAbsolute(endpoint) && !endpoint.includes("\0")) return endpoint
  let parsed
  try {
    parsed = new URL(endpoint)
  } catch (error) {
    fail("selection_invalid", "Project setup kernel endpoint is invalid", error)
  }
  if (!new Set(["ws:", "wss:"]).has(parsed.protocol)
    || !parsed.hostname
    || parsed.username
    || parsed.password
    || parsed.search
    || parsed.hash) {
    fail("selection_invalid", "Project setup kernel endpoint must be a WebSocket or absolute local socket path")
  }
  return parsed.toString()
}

function parseSelection(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)
    || value.schema !== PROJECT_SETUP_SELECTION_SCHEMA) {
    fail("selection_missing", "Project setup evidence has no reviewed product-observation selection")
  }
  const selectionKeys = [
    "schema", "kernel_endpoint", "kernel_id", "machine_id", "session_id", "agent_id", "project_id",
    "operation_id", "attempt", "created_at_ms", "worker_id", "worker_kernel_id", "platform",
  ]
  if (!exactKeys(value, selectionKeys)) {
    fail("selection_invalid", "Project setup selection has unsupported or missing fields")
  }

  const endpoint = parseEndpoint(value.kernel_endpoint)
  const selection = {
    endpoint,
    kernel_id: requiredIdentifier(value.kernel_id, "kernel ID"),
    machine_id: requiredIdentifier(value.machine_id, "machine ID"),
    session_id: requiredIdentifier(value.session_id, "session ID"),
    agent_id: requiredIdentifier(value.agent_id, "agent ID"),
    project_id: requiredIdentifier(value.project_id, "Project ID"),
    operation_id: requiredIdentifier(value.operation_id, "operation ID"),
    attempt: value.attempt,
    created_at_ms: requiredTime(value.created_at_ms, "operation creation time"),
    worker_id: requiredIdentifier(value.worker_id, "worker ID"),
    platform: typeof value.platform === "string" && value.platform.trim()
      ? value.platform
      : fail("selection_invalid", "Project setup selection is missing a validation platform"),
  }
  if (!Number.isSafeInteger(selection.attempt) || selection.attempt < 1) {
    fail("selection_invalid", "Project setup selection is missing a valid operation attempt")
  }
  selection.worker_kernel_id = requiredIdentifier(value.worker_kernel_id, "worker kernel ID")
  return selection
}

function unwrapVariant(value, variant, operation) {
  const result = value?.[variant]
  if (!result || value.error != null) fail("kernel_response_invalid", `kernel ${operation} did not return ${variant}`)
  return result
}

function unwrapAnyVariant(value, variants, operation) {
  if (!value || value.error != null) fail("kernel_response_invalid", `kernel ${operation} returned an error`)
  for (const variant of variants) {
    if (value?.[variant]) return value[variant]
  }
  fail("kernel_response_invalid", `kernel ${operation} did not return ${variants.join(" or ")}`)
}

function verifyKernelIdentity(envelope, selection) {
  const status = unwrapVariant(envelope, "RelayStatus", "identity")?.status
  if (!status || status.daemon_id !== selection.kernel_id || status.machine_id !== selection.machine_id) {
    fail("kernel_identity_mismatch", "selected kernel identity does not match the live kernel API response")
  }
  return { kernel_id: status.daemon_id, machine_id: status.machine_id }
}

function verifySession(envelope, selection) {
  const payload = unwrapAnyVariant(envelope, ["SessionState", "SessionStateLoaded"], "session state")
  const session = payload.session
  if (!session || session.id !== selection.session_id || session.project_id !== selection.project_id) {
    fail("session_identity_mismatch", "live kernel session does not match the selected session and Project")
  }
  if (!Array.isArray(session.agents)) fail("agent_identity_mismatch", "live kernel session has no agent inventory")
  const agent = session.agents.find((candidate) => candidate?.id === selection.agent_id)
  if (!agent) fail("agent_identity_mismatch", "selected agent is not present in the live kernel session")
  const remote = agent.remote_execution
  if (remote?.worker_machine_id !== selection.worker_id
    || remote?.worker_kernel_id !== selection.worker_kernel_id) {
    fail("agent_worker_mismatch", "selected agent is not bound to the selected worker and kernel")
  }
  return { session, agent }
}

function verifyProject(envelope, selection) {
  const projects = unwrapVariant(envelope, "ProjectsListed", "Project list")?.projects
  if (!Array.isArray(projects)) fail("project_identity_mismatch", "live kernel Project list is malformed")
  const matches = projects.filter((project) => project?.id === selection.project_id)
  if (matches.length !== 1 || matches[0].status !== "active") {
    fail("project_identity_mismatch", "selected Project is not uniquely active in the live kernel")
  }
  const definition = matches[0].environment_definition
  if (!definition || typeof definition !== "object" || Array.isArray(definition)) {
    fail("definition_identity_missing", "live Project has no stored environment definition")
  }
  if (!Array.isArray(definition.validation_commands) || definition.validation_commands.length === 0) {
    fail("definition_validation_missing", "live Project definition has no validation commands")
  }
  if (!new Set(["user_authored", "utility_generated"]).has(definition.origin)) {
    fail("definition_origin_missing", "live Project definition has no supported origin")
  }
  if (!DEFINITION_SOURCES.has(definition.source)) {
    fail("definition_source_missing", "live Project definition has no source")
  }
  if (definition.target_platform !== selection.platform) {
    fail("definition_platform_mismatch", "live Project definition targets a different platform")
  }
  // The kernel definition digest hashes serde's struct-order JSON bytes. The
  // API response preserves that field order, so do not sort these keys.
  const digest = fingerprint(JSON.stringify(definition))
  return { project: matches[0], definition, digest }
}

function validateSetupStatus(status, selection, definitionDigest, nowMs) {
  if (!status || typeof status !== "object" || Array.isArray(status)) {
    fail("operation_status_missing", "live kernel returned no Project setup status")
  }
  if (status.operation_id !== selection.operation_id
    || status.attempt !== selection.attempt
    || status.created_at_ms !== selection.created_at_ms) {
    fail("operation_identity_mismatch", "live Project setup status does not match the selected current operation")
  }
  if (status.project_id !== selection.project_id
    || status.session_id !== selection.session_id
    || status.agent_id !== selection.agent_id) {
    fail("operation_binding_mismatch", "live Project setup status changed its Project, session, or agent binding")
  }
  if (status.worker_id !== selection.worker_id || status.platform !== selection.platform) {
    fail("validation_binding_mismatch", "live Project setup status changed its worker or platform binding")
  }
  if (status.phase !== "ready" || status.progress_percent !== 100) {
    fail("setup_not_ready", "live Project setup operation has not reached Ready")
  }
  if (!Number.isSafeInteger(status.updated_at_ms)
    || status.updated_at_ms < status.created_at_ms
    || status.updated_at_ms > nowMs
    || nowMs - status.updated_at_ms > PROJECT_SETUP_MAX_STATUS_AGE_MS) {
    fail("operation_status_stale", "live Project setup Ready status is stale or has invalid timestamps")
  }
  if (typeof status.definition_digest !== "string" || !SHA256.test(status.definition_digest)) {
    fail("definition_identity_missing", "live Project setup status has no valid definition digest")
  }
  if (!status.validation || !Array.isArray(status.validation.commands)
    || status.validation.commands.length === 0) {
    fail("validation_missing", "live Project setup Ready status has no validation results")
  }
  if (status.validation.worker_id !== selection.worker_id
    || status.validation.platform !== selection.platform) {
    fail("validation_binding_mismatch", "live Project setup validation does not match the selected worker and platform")
  }
  if (status.definition_digest !== definitionDigest) {
    fail("definition_identity_mismatch", "live Project setup status does not match the selected Project definition identity")
  }
}

function snapshotDigest({ kernel, session, agent, project, status }) {
  return fingerprint(stableJson({
    kernel: { kernel_id: kernel.kernel_id, machine_id: kernel.machine_id },
    session: { id: session.id, project_id: session.project_id },
    agent: {
      id: agent.id,
      worker_machine_id: agent.remote_execution?.worker_machine_id,
      worker_kernel_id: agent.remote_execution?.worker_kernel_id,
    },
    project: {
      id: project.id,
      status: project.status,
      environment_definition: project.environment_definition,
    },
    setup_status: status,
  }))
}

function sameKernelIdentity(left, right) {
  return left.kernel_id === right.kernel_id && left.machine_id === right.machine_id
}

async function defaultDependencies() {
  const [ipcModule, requestModule] = await Promise.all([
    import(new URL("../../../../packages/kernel-client/dist/ipc.js", NODE_URL)),
    import(new URL("../../../../packages/kernel-client/dist/ipc-requests.js", NODE_URL)),
  ])
  if (typeof ipcModule.LocalIpcClient !== "function") {
    fail("kernel_client_missing", "built kernel public API client is unavailable")
  }
  return {
    clientFactory: (endpoint, options) => new ipcModule.LocalIpcClient(endpoint, options),
    requestBuilders: {
      relayStatusRequest: requestModule.relayStatusRequest,
      getSessionStateRequest: requestModule.getSessionStateRequest,
      listProjectsRequest: requestModule.listProjectsRequest,
      getProjectEnvironmentSetupStatusRequest: requestModule.getProjectEnvironmentSetupStatusRequest,
    },
  }
}

async function resolveDependencies(options) {
  const injected = Boolean(options.clientFactory || options.requestBuilders)
  const defaults = await defaultDependencies()
  return {
    ...options,
    clientFactory: options.clientFactory ?? defaults.clientFactory,
    requestBuilders: options.requestBuilders ?? defaults.requestBuilders,
    injected,
  }
}

function validateDependencies({ clientFactory, requestBuilders }) {
  if (typeof clientFactory !== "function") fail("kernel_client_missing", "kernel public API client factory is unavailable")
  for (const name of [
    "relayStatusRequest", "getSessionStateRequest", "listProjectsRequest", "getProjectEnvironmentSetupStatusRequest",
  ]) {
    if (typeof requestBuilders?.[name] !== "function") {
      fail("kernel_request_builder_missing", `kernel public API request builder ${name} is unavailable`)
    }
  }
}

function requestWithTimeout(client, request, label, deadline) {
  const remainingMs = deadline - Date.now()
  if (remainingMs <= 0) fail("observation_timeout", "Project setup product observation exceeded its time bound")
  const timeoutMs = Math.min(PROJECT_SETUP_REQUEST_TIMEOUT_MS, remainingMs)
  let timer
  return Promise.race([
    client.send(request),
    new Promise((_, reject) => {
      timer = setTimeout(() => reject(new ProjectSetupObserverError(
        "kernel_request_timeout",
        `kernel ${label} exceeded its time bound`,
      )), timeoutMs)
    }),
  ]).finally(() => clearTimeout(timer))
}

async function closeClient(client) {
  try {
    await client?.close?.()
  } catch {
    // Closing the public API transport is cleanup only.
  }
}

function statusFromEnvelope(envelope) {
  return unwrapVariant(envelope, "ProjectEnvironmentSetupStatus", "Project setup status")?.status
}

export async function observeManagedOrdinaryProjectSetup(evidence, {
  environment = process.env,
  clientFactory,
  requestBuilders,
  nowMs = Date.now,
} = {}) {
  if (!exactKeys(evidence, ["selection"])) {
    fail("caller_assertion_rejected", "Project setup input must contain only the product-observation selection")
  }
  const selection = parseSelection(evidence.selection)
  const relayAuthToken = typeof environment?.CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN === "string"
    ? environment.CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN.trim()
    : ""
  const dependencies = await resolveDependencies({ clientFactory, requestBuilders })
  validateDependencies(dependencies)

  const clientOptions = relayAuthToken
    ? { relayAuthToken, targetDaemonId: selection.kernel_id }
    : {}
  const client = dependencies.clientFactory(selection.endpoint, clientOptions)
  const deadline = Date.now() + PROJECT_SETUP_OBSERVATION_TIMEOUT_MS
  try {
    const observationStartedAtMs = nowMs()
    if (!Number.isSafeInteger(observationStartedAtMs) || observationStartedAtMs < 0) {
      fail("clock_invalid", "Project setup observer clock returned an invalid start timestamp")
    }
    const readKernel = async () => verifyKernelIdentity(
      await requestWithTimeout(client, dependencies.requestBuilders.relayStatusRequest(), "identity request", deadline),
      selection,
    )
    const readSession = async () => verifySession(
      await requestWithTimeout(
        client,
        dependencies.requestBuilders.getSessionStateRequest(selection.session_id),
        "session request",
        deadline,
      ),
      selection,
    )
    const readProject = async () => verifyProject(
      await requestWithTimeout(
        client,
        dependencies.requestBuilders.listProjectsRequest(false),
        "Project list request",
        deadline,
      ),
      selection,
    )
    const readStatus = async () => statusFromEnvelope(await requestWithTimeout(
      client,
      dependencies.requestBuilders.getProjectEnvironmentSetupStatusRequest(selection.operation_id),
      "Project setup status request",
      deadline,
    ))

    const initialKernel = await readKernel()
    const initialStatus = await readStatus()
    const initialSession = await readSession()
    const initialProject = await readProject()
    const finalSession = await readSession()
    const finalProject = await readProject()
    const finalStatus = await readStatus()
    const finalKernel = await readKernel()

    if (!sameKernelIdentity(initialKernel, finalKernel)) {
      fail("kernel_identity_changed", "kernel identity changed during Project setup observation")
    }
    if (stableJson(initialStatus) !== stableJson(finalStatus)) {
      fail("operation_status_changed", "Project setup status changed during product observation")
    }
    if (initialProject.digest !== finalProject.digest) {
      fail("definition_identity_changed", "stored Project definition changed during product observation")
    }
    if (initialSession.session.id !== finalSession.session.id
      || initialSession.agent.id !== finalSession.agent.id
      || initialSession.session.project_id !== finalSession.session.project_id
      || stableJson(initialSession.agent.remote_execution) !== stableJson(finalSession.agent.remote_execution)) {
      fail("session_identity_changed", "session or agent binding changed during Project setup observation")
    }

    const observationFinishedAtMs = nowMs()
    if (!Number.isSafeInteger(observationFinishedAtMs)
      || observationFinishedAtMs < observationStartedAtMs) {
      fail("clock_invalid", "Project setup observer clock returned an invalid finish timestamp")
    }
    validateSetupStatus(finalStatus, selection, finalProject.digest, observationFinishedAtMs)

    const { assertReadySetupValidation } = await import("./project-environment-setup-drill.mjs")
    const expected = {
      operation_id: selection.operation_id,
      project_id: selection.project_id,
      session_id: selection.session_id,
      agent_id: selection.agent_id,
      worker_id: selection.worker_id,
      platform: selection.platform,
    }
    try {
      assertReadySetupValidation(finalStatus, {
        expected,
        commands: finalProject.definition.validation_commands,
      })
    } catch (error) {
      fail("validation_incomplete", "live Project setup validation does not satisfy the shared Ready contract", error)
    }

    const beforeSnapshotDigest = snapshotDigest({
      kernel: initialKernel,
      session: initialSession.session,
      agent: initialSession.agent,
      project: initialProject.project,
      status: initialStatus,
    })
    const afterSnapshotDigest = snapshotDigest({
      kernel: finalKernel,
      session: finalSession.session,
      agent: finalSession.agent,
      project: finalProject.project,
      status: finalStatus,
    })
    const workerIdentityFingerprint = fingerprint(finalStatus.worker_id)
    const workerKernelIdentityFingerprint = fingerprint(finalSession.agent.remote_execution.worker_kernel_id)
    const proof = {
      schema: PROJECT_SETUP_PROOF_SCHEMA,
      product_api_observation: dependencies.injected ? "injected-test-transport" : "kernel-public-api",
      ready_validation_verified: true,
      status_fresh: true,
      before_after_identity_stable: beforeSnapshotDigest === afterSnapshotDigest,
      home_kernel_identity_fingerprint: fingerprint(`${finalKernel.kernel_id}\0${finalKernel.machine_id}`),
      session_identity_fingerprint: fingerprint(finalSession.session.id),
      agent_identity_fingerprint: fingerprint(finalSession.agent.id),
      project_identity_fingerprint: fingerprint(finalProject.project.id),
      operation_identity_fingerprint: fingerprint(finalStatus.operation_id),
      operation_attempt: finalStatus.attempt,
      operation_created_at_ms: finalStatus.created_at_ms,
      status_updated_at_ms: finalStatus.updated_at_ms,
      observation_started_at_ms: observationStartedAtMs,
      observation_finished_at_ms: observationFinishedAtMs,
      worker_identity_fingerprint: workerIdentityFingerprint,
      worker_kernel_identity_fingerprint: workerKernelIdentityFingerprint,
      target_identity_digest: projectSetupTargetIdentityDigest(workerIdentityFingerprint, workerKernelIdentityFingerprint, finalStatus.platform),
      platform: finalStatus.platform,
      validation_command_count: finalStatus.validation.commands.length,
      validation_receipts: finalStatus.validation.commands.map((entry) => ({
        command_digest: entry.command_digest,
        exit_code: entry.exit_code,
        stdout_bytes: entry.stdout_bytes,
        stderr_bytes: entry.stderr_bytes,
      })),
      definition_digest: finalStatus.definition_digest,
      definition_origin: finalProject.definition.origin,
      definition_source: finalProject.definition.source,
      definition_identity_verified: true,
      before_snapshot_digest: beforeSnapshotDigest,
      after_snapshot_digest: afterSnapshotDigest,
      transport_kind: dependencies.injected
        ? "injected-test-transport"
        : relayAuthToken ? "relay" : "kernel-public-api",
      endpoint_fingerprint: fingerprint(selection.endpoint),
    }
    if (!dependencies.injected) assertProjectSetupProof(proof)
    return proof
  } catch (error) {
    if (error instanceof ProjectSetupObserverError) throw error
    fail("product_observation_failed", "Project setup could not be observed through the kernel public API", error)
  } finally {
    await closeClient(client)
  }
}
