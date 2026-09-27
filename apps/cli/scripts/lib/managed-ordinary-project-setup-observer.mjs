import { createHash } from "node:crypto"
import { isAbsolute } from "node:path"

export const PROJECT_SETUP_SELECTION_SCHEMA = "chariox.managed-ordinary-project-setup-selection/v1"
export const PROJECT_SETUP_OBSERVER_SCHEMA = "chariox.managed-ordinary-project-setup-observer/v1"
export const PROJECT_SETUP_MAX_STATUS_AGE_MS = 15 * 60 * 1000
export const PROJECT_SETUP_OBSERVATION_TIMEOUT_MS = 60 * 1000
export const PROJECT_SETUP_REQUEST_TIMEOUT_MS = 10 * 1000

const IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9_.:/-]{0,127}$/
const SHA256 = /^sha256:[0-9a-f]{64}$/i
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
  if (value.worker_kernel_id !== undefined && value.worker_kernel_id !== null) {
    selection.worker_kernel_id = requiredIdentifier(value.worker_kernel_id, "worker kernel ID")
  }
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
  if (selection.worker_kernel_id !== undefined) {
    const remote = agent.remote_execution
    if (remote?.worker_machine_id !== selection.worker_id
      || remote?.worker_kernel_id !== selection.worker_kernel_id) {
      fail("agent_worker_mismatch", "selected agent is not bound to the selected worker and kernel")
    }
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
  if (typeof definition.source !== "string" || !definition.source) {
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

function sameStatusIdentity(left, right) {
  const fields = [
    "operation_id", "project_id", "session_id", "agent_id", "worker_id", "platform", "phase",
    "attempt", "progress_percent", "definition_digest", "created_at_ms", "updated_at_ms", "validation",
  ]
  return fields.every((field) => stableJson(left?.[field]) === stableJson(right?.[field]))
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
  if (options.clientFactory && options.requestBuilders) {
    return { ...options, injected: true }
  }
  const defaults = await defaultDependencies()
  return {
    ...options,
    clientFactory: options.clientFactory ?? defaults.clientFactory,
    requestBuilders: options.requestBuilders ?? defaults.requestBuilders,
    injected: false,
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
  const selection = parseSelection(evidence?.selection)
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
    if (!sameStatusIdentity(initialStatus, finalStatus)) {
      fail("operation_status_changed", "Project setup status changed during product observation")
    }
    if (initialProject.digest !== finalProject.digest) {
      fail("definition_identity_changed", "stored Project definition changed during product observation")
    }
    if (initialSession.session.id !== finalSession.session.id
      || initialSession.agent.id !== finalSession.agent.id
      || initialSession.session.project_id !== finalSession.session.project_id) {
      fail("session_identity_changed", "session or agent binding changed during Project setup observation")
    }

    const observedAtMs = nowMs()
    if (!Number.isSafeInteger(observedAtMs) || observedAtMs < 0) {
      fail("clock_invalid", "Project setup observer clock returned an invalid timestamp")
    }
    validateSetupStatus(finalStatus, selection, finalProject.digest, observedAtMs)

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

    return {
      schema: PROJECT_SETUP_OBSERVER_SCHEMA,
      product_api_observation: dependencies.injected ? "injected-test-transport" : "kernel-public-api",
      ready_validation_verified: true,
      status_fresh: true,
      kernel_identity_fingerprint: fingerprint(`${finalKernel.kernel_id}\0${finalKernel.machine_id}`),
      session_identity_fingerprint: fingerprint(finalSession.session.id),
      agent_identity_fingerprint: fingerprint(finalSession.agent.id),
      project_identity_fingerprint: fingerprint(finalProject.project.id),
      operation_identity_fingerprint: fingerprint(finalStatus.operation_id),
      operation_attempt: finalStatus.attempt,
      operation_created_at_ms: finalStatus.created_at_ms,
      status_updated_at_ms: finalStatus.updated_at_ms,
      worker_identity_fingerprint: fingerprint(finalStatus.worker_id),
      worker_kernel_identity_fingerprint: finalSession.agent.remote_execution?.worker_kernel_id
        ? fingerprint(finalSession.agent.remote_execution.worker_kernel_id)
        : null,
      platform: finalStatus.platform,
      validation_command_count: finalStatus.validation.commands.length,
      validation_command_digests: finalStatus.validation.commands.map((entry) => entry.command_digest),
      definition_digest: finalStatus.definition_digest,
      definition_origin: finalProject.definition.origin,
      definition_source: finalProject.definition.source,
      definition_identity_verified: true,
      transport_kind: dependencies.injected
        ? "injected-test-transport"
        : relayAuthToken ? "relay" : "kernel-public-api",
      endpoint_fingerprint: fingerprint(selection.endpoint),
    }
  } catch (error) {
    if (error instanceof ProjectSetupObserverError) throw error
    fail("product_observation_failed", "Project setup could not be observed through the kernel public API", error)
  } finally {
    await closeClient(client)
  }
}
