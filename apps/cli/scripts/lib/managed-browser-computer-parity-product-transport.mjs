import { createRequire } from "node:module"
import { readdir, stat } from "node:fs/promises"
import { fileURLToPath } from "node:url"

const OPERATOR_ENDPOINT_ENV = "CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL"
const TARGET_KERNEL_ENV = "CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF"
const TARGET_MACHINE_ENV = "CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF"
const OPERATOR_CLIENT_ENV = "CHARIOX_MANAGED_PARITY_CLIENT_ID"
const OPERATOR_SESSION_ENV = "CHARIOX_MANAGED_PARITY_SESSION_ID"
const DISPLAY_CONNECT_TIMEOUT_MS = 10_000
const DISPLAY_READY_TIMEOUT_MS = 10_000
const PUBLIC_REQUEST_TIMEOUT_MS = 15_000
const SLICE_LIFECYCLE_TIMEOUT_MS = 600_000
const RESOURCE_TELEMETRY_TIMEOUT_MS = 10_000
const PERSISTENCE_TIMEOUT_MS = 30_000
const CLEANUP_INSPECTION_TIMEOUT_MS = 15_000
const DISCONNECT_TIMEOUT_MS = 5_000
const MANAGED_PARITY_SCHEMA = "chariox.browser_computer_m0_guard.v1"
// This is the released wire constant at the reviewed PR head. Production
// construction also binds the value exported by kernel-client; the literal is
// only the local fail-closed reference used when a test injects that seam.
export const MANAGED_BROWSER_COMPUTER_PARITY_PROTOCOL = 335

/**
 * Load the released public client modules at runtime. Keeping this seam
 * injectable lets source-only tests exercise the transport without creating
 * ignored dist output or weakening the production import boundary.
 */
export async function loadManagedBrowserComputerParityKernelClientModules() {
  const [{ LocalIpcClient }, requestApi, displayApi, protocolApi] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../../../../packages/kernel-client/dist/display-stream.js"),
    import("../../../../packages/kernel-client/dist/kernel-types.js"),
  ])
  return { LocalIpcClient, requestApi, displayApi, protocolApi }
}

/**
 * Build the reviewed parity transport from the released public kernel client.
 *
 * The operator supplies only non-secret target references here. LocalIpcClient
 * consumes the normal one-shot local-auth configuration itself, resolves a
 * scoped relay connection through the home kernel, and keeps the relay token
 * in memory. The home client remains open as the Room authority for display
 * admission; the scoped worker client supplies only target identity/status.
 * Every operation not mapped to a released public request remains fail-closed.
 */
export async function createManagedBrowserComputerParityTransport({
  evidenceRoot,
  signal,
  kernelClientModules,
  resourceTelemetry,
  persistence,
  cleanupInspector,
  config = null,
  operationAdapter = null,
  reconnectClient = null,
  timeouts,
} = {}) {
  if (typeof evidenceRoot !== "string" || evidenceRoot.trim() === "") {
    throw new Error("managed parity transport requires an external evidenceRoot")
  }

  const homeKernelUrl = requiredOperatorValue(OPERATOR_ENDPOINT_ENV)
  const targetKernelRef = requiredOperatorValue(TARGET_KERNEL_ENV)
  const targetMachineRef = requiredOperatorValue(TARGET_MACHINE_ENV)
  const clientId = requiredOperatorValue(OPERATOR_CLIENT_ENV)
  const sessionId = requiredOperatorValue(OPERATOR_SESSION_ENV)
  requireStandardLocalAuthConfiguration()

  const modules = kernelClientModules ?? await loadManagedBrowserComputerParityKernelClientModules()
  const LocalIpcClient = modules.LocalIpcClient ?? modules.ipc?.LocalIpcClient
  const requestApi = modules.requestApi ?? modules.ipcRequests
  const displayApi = modules.displayApi ?? modules.displayStream
  if (typeof LocalIpcClient !== "function" || !requestApi || !displayApi) {
    throw new Error("managed parity kernel-client modules are incomplete")
  }
  const homeClient = new LocalIpcClient(homeKernelUrl)
  let connection
  try {
    const response = await sendWithAbortSignal(
      homeClient,
      requestApi.resolveKernelClientConnectionRequest({
        kernelRef: targetKernelRef,
        machineRef: targetMachineRef,
        clientId,
        sessionId,
      }),
      signal,
      "resolve target",
    )
    connection = resolvedConnection(response)
  } catch (error) {
    await homeClient.close().catch(() => {})
    throw error
  }

  requireText(connection.relay_url, "KernelClientConnectionResolved.connection.relay_url")
  requireText(connection.relay_token, "KernelClientConnectionResolved.connection.relay_token")
  if (!connection.target_daemon_id && !connection.target_daemon_alias) {
    throw new Error("managed parity target did not return a daemon identity")
  }
  if (connection.target_daemon_id !== undefined && connection.target_daemon_id !== null) {
    requireText(connection.target_daemon_id, "KernelClientConnectionResolved.connection.target_daemon_id")
  }
  if (connection.target_daemon_alias !== undefined && connection.target_daemon_alias !== null) {
    requireText(connection.target_daemon_alias, "KernelClientConnectionResolved.connection.target_daemon_alias")
  }

  let workerClient
  try {
    workerClient = new LocalIpcClient(connection.relay_url, {
      relayAuthToken: connection.relay_token,
      targetDaemonId: connection.target_daemon_id ?? undefined,
      targetDaemonAlias: connection.target_daemon_alias ?? undefined,
    })
    const webSocketModule = modules.webSocket ?? createRequire(fileURLToPath(new URL(
      "../../../../packages/kernel-client/dist/ipc.js",
      import.meta.url,
    )))("ws")
    const webSocket = webSocketModule.WebSocket ?? webSocketModule.default ?? webSocketModule
    let activeWorkerClient = workerClient
    const reconnectFactory = reconnectClient
      ? async (input) => {
        const replacement = await reconnectClient(input)
        activeWorkerClient = replacement
        return replacement
      }
      : async () => {
        const replacement = new LocalIpcClient(connection.relay_url, {
          relayAuthToken: connection.relay_token,
          targetDaemonId: connection.target_daemon_id ?? undefined,
          targetDaemonAlias: connection.target_daemon_alias ?? undefined,
        })
        activeWorkerClient = replacement
        return replacement
      }
    if (!hasManagedResourceTelemetryPath(workerClient, requestApi, resourceTelemetry)) {
      throw new Error("managed parity transport requires a complete kernel-managed resource telemetry path")
    }
    return {
      ...createManagedBrowserComputerParityTransportFromPublicClient({
        client: workerClient,
        displayClient: homeClient,
        identityClient: workerClient,
        requestApi,
        targetKernelRef,
        targetMachineRef,
        resourceTelemetry,
        persistence,
        cleanupInspector,
        evidenceRoot,
        parityConfig: config,
        operationAdapter,
        reconnectClient: reconnectFactory,
        protocolApi: modules.protocolApi,
        timeouts,
        displayTransport: {
          openSelkiesDisplayStream: displayApi.openSelkiesDisplayStream,
          webSocket,
        },
      }),
      close: async () => {
        await activeWorkerClient.close().catch(() => {})
        await homeClient.close().catch(() => {})
      },
    }
  } catch (error) {
    await workerClient?.close().catch(() => {})
    await homeClient.close().catch(() => {})
    throw error
  }
}

/**
 * Compose the transport around the public client's request/response seam.
 * Tests use an externally controlled authenticated endpoint here; production
 * construction above supplies home/worker LocalIpcClient instances, the
 * released request module, and the released display-stream module.
 */
export function createManagedBrowserComputerParityTransportFromPublicClient({
  client,
  displayClient = client,
  identityClient = client,
  requestApi,
  targetKernelRef = null,
  targetMachineRef = null,
  displayTransport,
  resourceTelemetry = null,
  managedResourceTelemetry = null,
  resourceTelemetryAdapter = null,
  persistence = null,
  persistenceTransport = null,
  cleanupInspector = null,
  residueInspector = null,
  parityConfig = null,
  operationAdapter = null,
  reconnectClient = null,
  protocolApi = null,
  timeouts = {},
  evidenceRoot = null,
} = {}) {
  if (!client || typeof client.send !== "function") {
    throw new Error("managed parity transport requires a public kernel client")
  }
  if (!displayClient || typeof displayClient.send !== "function") {
    throw new Error("managed parity transport requires a public display client")
  }
  if (!identityClient || typeof identityClient.send !== "function") {
    throw new Error("managed parity transport requires a public identity client")
  }
  if (!requestApi || typeof requestApi.getSliceDisplayEndpointRequest !== "function") {
    throw new Error("managed parity transport requires released kernel request constructors")
  }
  if (displayTransport !== undefined && typeof displayTransport.openSelkiesDisplayStream !== "function") {
    throw new Error("managed parity transport requires a released Selkies display-stream opener")
  }
  if (displayTransport !== undefined && (typeof requestApi.relayStatusRequest !== "function"
    || typeof requestApi.getRoomEnvironmentStateRequest !== "function")) {
    throw new Error("managed parity transport requires released identity and Room request constructors")
  }

  const ownedResources = {
    sliceId: null,
    expectedRoomId: null,
    attachmentIds: new Set(),
    attachmentsByClient: new Map(),
    agentIds: new Set(),
    identity: null,
    stableIdentity: null,
    detachedAttachmentIds: new Set(),
    cleanupEvidence: null,
    cleanupMeasurements: null,
    evidenceRoot: hasText(evidenceRoot) ? evidenceRoot.trim() : null,
  }

  const activeClientRef = { current: client }
  const activeIdentityClientRef = { current: identityClient }

  const telemetryAdapter = resourceTelemetry ?? managedResourceTelemetry ?? resourceTelemetryAdapter
  const persistenceAdapter = persistence
    ?? persistenceTransport
    ?? (firstCallable(displayClient, ["describePersistenceMutations", "runPersistenceMutations"])
      ? displayClient
      : createKernelPersistenceAdapter({
        client: displayClient,
        identityClient: displayClient,
        requestApi,
      }))
  const residueAdapter = cleanupInspector
    ?? residueInspector
    ?? (firstCallable(identityClient, ["inspectCleanupResidue", "cleanupInspect"])
      ? identityClient
      : createKernelCleanupInspector({
        client: displayClient,
        identityClientRef: activeIdentityClientRef,
        requestApi,
        targetKernelRef,
        targetMachineRef,
        telemetryAdapter,
        ownedResources,
      }))
  const productionOperationAdapter = operationAdapter
    ?? createKernelOperationAdapter({
      clientRef: activeClientRef,
      identityClientRef: activeIdentityClientRef,
      client,
      displayClient,
      identityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      ownedResources,
      parityConfig,
      protocolApi,
      reconnectClient,
      telemetryAdapter,
    })
  const requiresCompatibilityPreflight = Boolean(parityConfig || protocolApi)
  let compatibilityResult = null
  const timeoutConfig = {
    resourceTelemetryMs: finiteTimeout(timeouts.resourceTelemetryMs, RESOURCE_TELEMETRY_TIMEOUT_MS),
    persistenceMs: finiteTimeout(timeouts.persistenceMs, PERSISTENCE_TIMEOUT_MS),
    cleanupInspectionMs: finiteTimeout(timeouts.cleanupInspectionMs, CLEANUP_INSPECTION_TIMEOUT_MS),
    sliceLifecycleMs: finiteTimeout(
      timeouts.sliceLifecycleMs,
      SLICE_LIFECYCLE_TIMEOUT_MS,
      SLICE_LIFECYCLE_TIMEOUT_MS,
    ),
  }

  const transport = {
    resourceScope: "managed-target",
    targetId: targetMachineRef ?? targetKernelRef ?? null,
    targetKernelRef,
    targetMachineRef,
    requiresCompatibilityPreflight,
    async assertCompatibilityPreflight({ signal, config = parityConfig } = {}) {
      if (!compatibilityResult) {
        compatibilityResult = await withDeadline(
          (deadlineSignal) => runCompatibilityPreflight({
            identityClient: activeIdentityClientRef.current,
            displayClient,
            requestApi,
            targetKernelRef,
            targetMachineRef,
            parityConfig: config,
            protocolApi,
            signal: deadlineSignal,
          }),
          { signal, timeoutMs: timeoutConfig.resourceTelemetryMs, step: "compatibility preflight" },
        )
      }
      return compatibilityResult
    },
    async collectManagedTargetResourceSnapshot({ phase, sampleId, evidenceRoot, now, signal } = {}) {
      if (requiresCompatibilityPreflight && !compatibilityResult) {
        throw new Error("managed parity resource telemetry requires compatibility preflight first")
      }
      return withDeadline(
        (deadlineSignal) => collectManagedTargetResourceSnapshot({
          client: activeIdentityClientRef.current,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          ownedResources,
          telemetryAdapter,
          phase,
          sampleId,
          evidenceRoot,
          now,
          signal: deadlineSignal,
        }),
        { signal, timeoutMs: timeoutConfig.resourceTelemetryMs, step: "resource telemetry" },
      )
    },
    async describePersistenceMutations(request, { signal } = {}) {
      return withDeadline(
        (deadlineSignal) => describePersistenceMutations({
          client: displayClient,
          identityClient: displayClient,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          ownedResources,
          persistenceAdapter,
          request,
          signal: deadlineSignal,
        }),
        { signal, timeoutMs: timeoutConfig.persistenceMs, step: "persistence description" },
      )
    },
    async run(step, request, { signal, onPersistenceMutation } = {}) {
      if (signal?.aborted) {
        throw new Error(`managed parity ${step} was aborted before the public request`)
      }
      if (requiresCompatibilityPreflight && step === "preflight") {
        await transport.assertCompatibilityPreflight({ signal })
      }
      if (requiresCompatibilityPreflight && step !== "preflight" && !compatibilityResult) {
        throw new Error(`managed parity ${step} requires compatibility preflight before any target operation`)
      }
      if (productionOperationAdapter && typeof productionOperationAdapter[step] === "function") {
        const result = await withDeadline(
          (deadlineSignal) => productionOperationAdapter[step]({
            request,
            signal: deadlineSignal,
            compatibility: compatibilityResult,
            onPersistenceMutation,
          }),
          {
            signal,
            timeoutMs: step === "selkies.persistence"
              ? timeoutConfig.persistenceMs
              : timeoutConfig.sliceLifecycleMs,
            step,
          },
        )
        if (["selkies.browser", "selkies.computer"].includes(step) && hasText(result?.agentId)) {
          ownedResources.agentIds.add(result.agentId.trim())
        }
        return result
      }
      if (step === "novnc.create") {
        return runDisplayBackendCreate({
          displayClient,
          identityClient,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          ownedResources,
          evidenceRoot,
          request,
          signal,
          displayBackend: "novnc",
          sliceLifecycleTimeoutMs: timeoutConfig.sliceLifecycleMs,
        })
      }
      if (step === "novnc.attach") {
        return runNovncAttach({
          displayClient,
          identityClient,
          requestApi,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "novnc.rollback") {
        return runNovncRollback({
          displayClient,
          requestApi,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "novnc.destroy") {
        return runSelkiesDestroy({
          displayClient,
          identityClient: activeIdentityClientRef.current,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          telemetryAdapter,
          ownedResources,
          request,
          signal,
          displayBackend: "novnc",
          sliceLifecycleTimeoutMs: timeoutConfig.sliceLifecycleMs,
        })
      }
      if (step === "selkies.create") {
        return runSelkiesCreate({
          displayClient,
          identityClient,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          ownedResources,
          evidenceRoot,
          request,
          signal,
          sliceLifecycleTimeoutMs: timeoutConfig.sliceLifecycleMs,
        })
      }
      if (step === "selkies.attach") {
        return runSelkiesAttach({
          displayClient,
          identityClient,
          requestApi,
          displayTransport,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "selkies.persistence") {
        return withDeadline(
          (deadlineSignal) => runPersistenceMutations({
            client: displayClient,
            identityClient: displayClient,
            requestApi,
            targetKernelRef,
            targetMachineRef,
            ownedResources,
            persistenceAdapter,
            request,
            onPersistenceMutation,
            signal: deadlineSignal,
          }),
          { signal, timeoutMs: timeoutConfig.persistenceMs, step: "persistence execution" },
        )
      }
      if (step === "selkies.destroy") {
        return runSelkiesDestroy({
          displayClient,
          identityClient: activeIdentityClientRef.current,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          telemetryAdapter,
          ownedResources,
          request,
          signal,
          displayBackend: "selkies",
          sliceLifecycleTimeoutMs: timeoutConfig.sliceLifecycleMs,
        })
      }
      if (step === "cleanup.perform") {
        return runCleanup({
          displayClient,
          identityClient: activeIdentityClientRef.current,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          telemetryAdapter,
          ownedResources,
          signal,
          sliceLifecycleTimeoutMs: timeoutConfig.sliceLifecycleMs,
        })
      }
      if (step === "cleanup.inspect") {
        return withDeadline(
          (deadlineSignal) => inspectCleanupResidue({
            displayClient,
            requestApi,
            ownedResources,
            residueAdapter,
            request,
            signal: deadlineSignal,
          }),
          { signal, timeoutMs: timeoutConfig.cleanupInspectionMs, step: "cleanup inspection" },
        )
      }
      throw new Error(`unsupported managed parity step: ${step}`)
    },
  }

  return transport
}

function createKernelPersistenceAdapter({ client, identityClient, requestApi }) {
  if (typeof requestApi?.saveSliceStateRequest !== "function"
    || typeof requestApi?.stopSliceRequest !== "function"
    || typeof requestApi?.startSliceRequest !== "function"
    || typeof requestApi?.getSliceStateStatusRequest !== "function"
    || typeof requestApi?.getSliceRequest !== "function"
    || typeof requestApi?.getRoomEnvironmentResourceInventoryRequest !== "function") {
    return null
  }
  return {
    async describe(input) {
      return createKernelPersistencePlan(input, requestApi)
    },
    async run(input) {
      return executeKernelPersistence({
        ...input,
        // runPersistenceMutations supplies display-client wrappers so the
        // request ledger captures the exact awaited public lifecycle replies.
        // Those wrappers are already selected from the home/display client;
        // never fall back to the target-worker client here.
        client: input?.client ?? client,
        identityClient: input?.identityClient ?? identityClient,
        requestApi,
      })
    },
  }
}

const PERSISTENCE_MUTATION_ACTIONS = Object.freeze(["save", "remove", "restore"])

const PERSISTENCE_MUTATION_REQUEST_VARIANTS = Object.freeze({
  save: "SaveSliceState",
  remove: "StopSlice",
  restore: "StartSlice",
})

const PERSISTENCE_MUTATION_RESPONSE_VARIANTS = Object.freeze({
  save: "SliceStateSaved",
  remove: "SliceStopped",
  restore: "SliceStarted",
})

const PERSISTENCE_MUTATION_CHECKPOINTS = Object.freeze({
  save: Object.freeze({ before: "before-docker-save", after: "after-docker-save" }),
  remove: Object.freeze({ before: "before-docker-remove", after: "after-docker-remove" }),
  restore: Object.freeze({ before: "before-docker-restore", after: "after-docker-restore" }),
})

function createKernelPersistencePlan(input, requestApi) {
  const sliceId = requireText(input?.ownedResource?.sliceId, "persistence owned slice id")
  const requests = {
    save: requireRequestConstructor(requestApi, "saveSliceStateRequest")(
      sliceId,
      "shutdown",
      "this_slice",
    ),
    remove: requireRequestConstructor(requestApi, "stopSliceRequest")(sliceId),
    restore: requireRequestConstructor(requestApi, "startSliceRequest")(sliceId),
  }
  const plan = {
    schema: MANAGED_PARITY_SCHEMA,
    persistenceMutations: PERSISTENCE_MUTATION_ACTIONS.map((action) => (
      createPersistenceMutationDefinition(action, requests[action])
    )),
    ownedResource: redactManagedValue(input?.ownedResource ?? null),
  }
  return deepFreeze(plan)
}

function createPersistenceMutationDefinition(action, request) {
  const requestVariant = PERSISTENCE_MUTATION_REQUEST_VARIANTS[action]
  if (!request || typeof request !== "object" || Array.isArray(request)
    || Object.keys(request).length !== 1 || !Object.hasOwn(request, requestVariant)) {
    throw new Error(`managed parity persistence ${action} request must be the exact public ${requestVariant} request`)
  }
  const normalizedRequest = redactManagedValue(request)
  const argv = persistenceRequestArgv(action, normalizedRequest)
  return {
    action,
    argv,
    request: normalizedRequest,
    requestIdentity: persistenceRequestIdentity(normalizedRequest),
    responseVariant: PERSISTENCE_MUTATION_RESPONSE_VARIANTS[action],
    checkpoints: PERSISTENCE_MUTATION_CHECKPOINTS[action],
  }
}

function persistenceRequestArgv(action, request) {
  const requestVariant = PERSISTENCE_MUTATION_REQUEST_VARIANTS[action]
  const payload = request?.[requestVariant]
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    throw new Error(`managed parity persistence ${action} request payload is malformed`)
  }
  const sliceId = requireText(payload.slice_ref, `${requestVariant}.slice_ref`)
  const argv = ["kernel", requestVariant, sliceId]
  if (action === "save") {
    for (const field of ["mode", "scope"]) {
      if (payload[field] !== undefined && payload[field] !== null) {
        if (typeof payload[field] !== "string" || payload[field].trim() === "") {
          throw new Error(`managed parity persistence ${action} request ${field} is malformed`)
        }
        argv.push(`${field}=${payload[field]}`)
      }
    }
  }
  return requireSafeArgv(argv, `persistence ${action}`)
}

function persistenceRequestIdentity(request) {
  return JSON.stringify(request)
}

function persistenceReceiptId(action, requestIdentity, responseSlice) {
  return `${action}:${responseSlice.id}:${requestIdentity}`
}

function requirePersistenceResponseSlice(payload, sliceId, step) {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    throw new Error(`managed parity ${step} response payload is malformed`)
  }
  const slice = payload.slice
  if (!slice || typeof slice !== "object" || slice.id !== sliceId) {
    throw new Error(`managed parity ${step} response returned a different slice identity`)
  }
  return slice
}

function requireSavedState(value, sliceId, step) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`managed parity ${step} response did not return a saved state`)
  }
  if (value.source_slice_id !== sliceId) {
    throw new Error(`managed parity ${step} saved state belongs to a different slice`)
  }
  requireText(value.id, `${step}.state.id`)
  requireText(value.home_archive_path, `${step}.state.home_archive_path`)
  return value
}

function validatePersistenceTransition({
  action,
  before,
  after,
  response,
  responseVariant,
  sliceId,
  savedState,
}) {
  if (!before?.slice || !after?.slice) {
    throw new Error(`managed parity persistence ${action} requires authoritative before and after slice state`)
  }
  if (before.slice.id !== sliceId || after.slice.id !== sliceId) {
    throw new Error(`managed parity persistence ${action} changed slice identity`)
  }
  if (!samePersistenceObservationIdentity(before, after)) {
    throw new Error(`managed parity persistence ${action} changed its authoritative Room or resource identity`)
  }
  const expectedStatuses = {
    save: { before: "running", after: "stopped" },
    remove: { before: "stopped", after: "stopped" },
    restore: { before: "stopped", after: "running" },
  }[action]
  if (before.slice.status !== expectedStatuses.before || after.slice.status !== expectedStatuses.after) {
    throw new Error(
      `managed parity persistence ${action} observed an invalid authoritative slice transition: `
      + `${before.slice.status} -> ${after.slice.status}`,
    )
  }
  const responseSlice = requirePersistenceResponseSlice(response, sliceId, `persistence ${action}`)
  if (responseSlice.status !== after.slice.status) {
    throw new Error(`managed parity persistence ${action} response slice state differs from its authoritative after state`)
  }
  if (responseVariant === PERSISTENCE_MUTATION_RESPONSE_VARIANTS.save) {
    const state = requireSavedState(response.state, sliceId, `persistence ${action}`)
    if (state.id !== savedState?.id) {
      throw new Error("managed parity persistence save response changed saved-state identity")
    }
  }
  if (responseVariant === PERSISTENCE_MUTATION_RESPONSE_VARIANTS.restore
    && savedState?.id !== undefined
    && after.slice.saved_state_ref !== undefined
    && after.slice.saved_state_ref !== null
    && after.slice.saved_state_ref !== savedState.id) {
    throw new Error("managed parity persistence restore authoritative slice references a stale saved state")
  }
}

function samePersistenceObservationIdentity(left, right) {
  if (!(left?.slice?.id === right?.slice?.id
    && left?.slice?.environment_session_id === right?.slice?.environment_session_id
    && left?.slice?.environment_id === right?.slice?.environment_id)) {
    return false
  }
  if (!left?.inventory || !right?.inventory) return true
  return left.inventory.session_id === right.inventory.session_id
    && left.inventory.environment_id === right.inventory.environment_id
    && left.inventory.slice_id === right.inventory.slice_id
    && sameArray(left.inventory.profile_ids, right.inventory.profile_ids)
}

async function executeKernelPersistence({
  client,
  identityClient,
  requestApi,
  ownedResource,
  request,
  plan,
  onPersistenceMutation,
  signal,
}) {
  const sliceId = requireText(ownedResource?.sliceId, "persistence owned slice id")
  const planned = plan?.persistenceMutations
  if (!Array.isArray(planned) || planned.length !== 3) {
    throw new Error("managed parity persistence execution requires an immutable kernel plan")
  }
  const before = []
  const receipts = []
  const executed = []
  let savedState = null
  let initialInventory = null
  let finalInventory = null
  let previousAfterInventory = null
  for (const [index, mutation] of planned.entries()) {
    const beforeInventory = await readKernelPersistenceInventory({
      client: identityClient ?? client,
      requestApi,
      sliceId,
      roomId: ownedResource?.roomId,
      signal,
      step: `persistence ${mutation.action} inventory`,
    })
    if (previousAfterInventory && !samePersistenceObservationIdentity(previousAfterInventory, beforeInventory)) {
      throw new Error(`managed parity persistence ${mutation.action} did not continue from the previous authoritative slice state`)
    }
    before.push(beforeInventory)
    if (index === 0) initialInventory = beforeInventory
    await onPersistenceMutation?.({
      phase: "before",
      mutation: redactManagedValue({ ...mutation, before: beforeInventory }),
    })
    const requestIdentity = persistenceRequestIdentity(mutation.request)
    if (mutation.requestIdentity !== requestIdentity) {
      throw new Error(`managed parity persistence ${mutation.action} request identity changed after planning`)
    }
    let response
    let responseVariantName
    let responsePayload
    let receipt
    if (mutation.action === "save") {
      response = await sendWithAbortSignal(
        client,
        mutation.request,
        signal,
        "persistence save",
      )
      responseVariantName = PERSISTENCE_MUTATION_RESPONSE_VARIANTS.save
      responsePayload = responseVariant(response, responseVariantName, "persistence save")
      const saved = responsePayload
      const responseSlice = requirePersistenceResponseSlice(saved, sliceId, "persistence save")
      savedState = requireSavedState(saved.state, sliceId, "persistence save")
      const archivePath = requireText(savedState?.home_archive_path, "SliceStateSaved.state.home_archive_path")
      receipt = {
        ok: true,
        id: savedState.id,
        action: mutation.action,
        requestIdentity,
        responseVariant: responseVariantName,
        responseSliceId: responseSlice.id,
        savedStateId: savedState.id,
        archivePath,
        authoritative: true,
      }
    } else if (mutation.action === "remove") {
      response = await sendWithAbortSignal(
        client,
        mutation.request,
        signal,
        "persistence remove",
      )
      responseVariantName = PERSISTENCE_MUTATION_RESPONSE_VARIANTS.remove
      responsePayload = responseVariant(response, responseVariantName, "persistence remove")
      const responseSlice = requirePersistenceResponseSlice(responsePayload, sliceId, "persistence remove")
      receipt = {
        ok: true,
        id: persistenceReceiptId(mutation.action, requestIdentity, responseSlice),
        action: mutation.action,
        requestIdentity,
        responseVariant: responseVariantName,
        responseSliceId: responseSlice.id,
        savedStateId: requireText(savedState?.id, "persistence remove saved state id"),
        parentReceiptId: receiptId(receipts[0]),
        archivePath: requireText(savedState?.home_archive_path, "saved state archive path"),
        authoritative: true,
      }
    } else {
      response = await sendWithAbortSignal(
        client,
        mutation.request,
        signal,
        "persistence restore",
        SLICE_LIFECYCLE_TIMEOUT_MS,
      )
      responseVariantName = PERSISTENCE_MUTATION_RESPONSE_VARIANTS.restore
      responsePayload = responseVariant(response, responseVariantName, "persistence restore")
      const responseSlice = requirePersistenceResponseSlice(responsePayload, sliceId, "persistence restore")
      const statusResponse = await sendWithAbortSignal(
        client,
        requestApi.getSliceStateStatusRequest(sliceId),
        signal,
        "persistence restore state status",
      )
      const status = responseVariant(statusResponse, "SliceStateStatus", "persistence restore state status")
      if (!status.slice || status.slice.id !== sliceId) {
        throw new Error("managed parity persistence restore state status returned a different slice identity")
      }
      if (!status.state || status.state.id !== savedState?.id) {
        throw new Error("managed parity persistence restore did not restore the authoritative saved state")
      }
      receipt = {
        ok: true,
        id: persistenceReceiptId(mutation.action, requestIdentity, responseSlice),
        action: mutation.action,
        requestIdentity,
        responseVariant: responseVariantName,
        responseSliceId: responseSlice.id,
        savedStateId: status.state.id,
        parentReceiptId: receiptId(receipts[1]),
        archivePath: requireText(savedState?.home_archive_path, "saved state archive path"),
        authoritative: true,
      }
    }
    const afterInventory = await readKernelPersistenceInventory({
      client: identityClient ?? client,
      requestApi,
      sliceId,
      roomId: ownedResource?.roomId,
      signal,
      step: `persistence ${mutation.action} after inventory`,
    })
    validatePersistenceTransition({
      action: mutation.action,
      before: beforeInventory,
      after: afterInventory,
      response: responsePayload,
      responseVariant: responseVariantName,
      sliceId,
      savedState,
    })
    previousAfterInventory = afterInventory
    const evidence = {
      ...mutation,
      before: beforeInventory,
      response: {
        variant: responseVariantName,
        payload: redactManagedValue(responsePayload),
      },
      after: afterInventory,
      receipt,
      ...(index > 0 ? { saveReceipt: receipts[0] } : {}),
      ...(index > 1 ? { removeReceipt: receipts[1] } : {}),
    }
    receipts.push(receipt)
    executed.push(evidence)
    await onPersistenceMutation?.({ phase: "after", mutation: redactManagedValue(evidence) })
    if (mutation.action === "restore") {
      finalInventory = afterInventory
    }
  }
  if (!initialInventory || !finalInventory) {
    throw new Error("managed parity persistence did not observe both initial and restored inventory")
  }
  const sameRoom = initialInventory.slice?.environment_session_id === finalInventory.slice?.environment_session_id
    && initialInventory.slice?.environment_session_id === ownedResource?.roomId
  const sameEnvironment = initialInventory.inventory?.environment_id === finalInventory.inventory?.environment_id
    && hasText(initialInventory.inventory?.environment_id)
  const sameProfile = sameArray(
    initialInventory.inventory?.profile_ids,
    finalInventory.inventory?.profile_ids,
  ) && finalInventory.inventory.profile_ids.length === 1
  if (!sameRoom || !sameEnvironment || !sameProfile) {
    throw new Error("managed parity persistence restore changed the authoritative Room, environment, or profile identity")
  }
  return {
    schema: MANAGED_PARITY_SCHEMA,
    ...redactManagedValue(request?.binding ?? {}),
    binding: redactManagedValue(request?.binding ?? null),
    saved: true,
    restarted: true,
    sameRoom,
    sameEnvironment,
    sameProfile,
    persistenceMutations: executed.map((mutation) => redactManagedValue(mutation)),
  }
}

async function readKernelPersistenceInventory({ client, requestApi, sliceId, roomId, signal, step }) {
  const sliceResponse = await sendWithAbortSignal(
    client,
    requestApi.getSliceRequest(sliceId),
    signal,
    `${step} slice`,
  )
  const slice = responseVariant(sliceResponse, "Slice", `${step} slice`).slice
  if (!slice || slice.id !== sliceId) throw new Error(`${step} returned a different slice identity`)
  const resolvedRoomId = roomId ?? slice.environment_session_id ?? slice.session_id
  if (!hasText(resolvedRoomId)) throw new Error(`${step} returned no Room identity`)
  if ((slice.environment_session_id ?? slice.session_id) !== resolvedRoomId) {
    throw new Error(`${step} returned a different Room identity`)
  }
  if (slice.status === "stopped") {
    return {
      slice: redactManagedValue({
        id: slice.id,
        status: slice.status,
        environment_session_id: slice.environment_session_id ?? slice.session_id ?? null,
        environment_id: slice.environment_id ?? null,
        saved_state_ref: slice.saved_state_ref ?? null,
        saved_state_status: slice.saved_state_status ?? null,
        last_operation: slice.last_operation ?? null,
        last_operation_status: slice.last_operation_status ?? null,
      }),
      inventory: null,
    }
  }
  const inventoryResponse = await sendWithAbortSignal(
    client,
    requestApi.getRoomEnvironmentResourceInventoryRequest(resolvedRoomId, sliceId),
    signal,
    `${step} browser/profile inventory`,
  )
  const inventory = responseVariant(
    inventoryResponse,
    "RoomEnvironmentResourceInventory",
    `${step} browser/profile inventory`,
  ).inventory
  if (!inventory || typeof inventory !== "object" || Array.isArray(inventory)
    || inventory.slice_id !== sliceId
    || inventory.session_id !== resolvedRoomId
    || !hasText(inventory.environment_id)) {
    throw new Error(`${step} returned a different authoritative resource identity`)
  }
  requireUniqueIdentityArray(inventory?.profile_ids, `${step} profile_ids`)
  requireUniqueIdentityArray(inventory?.browser_ids, `${step} browser_ids`)
  return {
    slice: redactManagedValue({
      id: slice.id,
      status: slice.status,
      environment_session_id: slice.environment_session_id ?? slice.session_id ?? null,
      environment_id: slice.environment_id ?? null,
      saved_state_ref: slice.saved_state_ref ?? null,
      saved_state_status: slice.saved_state_status ?? null,
      last_operation: slice.last_operation ?? null,
      last_operation_status: slice.last_operation_status ?? null,
    }),
    inventory: redactManagedValue({
      environment_id: inventory.environment_id,
      slice_id: inventory.slice_id,
      browser_ids: inventory.browser_ids,
      profile_ids: inventory.profile_ids,
    }),
  }
}

function createKernelCleanupInspector({
  client,
  identityClientRef,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  telemetryAdapter,
  ownedResources,
}) {
  if (typeof client?.send !== "function"
    || typeof requestApi?.listSlicesRequest !== "function"
    || typeof requestApi?.listSessionsRequest !== "function"
    || typeof requestApi?.getRoomEnvironmentSliceRequest !== "function"
    || typeof requestApi?.getRoomEnvironmentResourceInventoryRequest !== "function"
    || typeof requestApi?.getRoomEnvironmentStateRequest !== "function"
    || !hasManagedResourceTelemetryPath(identityClientRef?.current ?? client, requestApi, telemetryAdapter)) {
    return null
  }
  return {
    async inspect(input) {
      return inspectMeasuredKernelResidue({
        client,
        identityClient: identityClientRef?.current ?? client,
        requestApi,
        targetKernelRef,
        targetMachineRef,
        telemetryAdapter,
        ownedResources,
        ...input,
      })
    },
  }
}

async function inspectMeasuredKernelResidue({
  client,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  telemetryAdapter,
  ownedResources,
  cleanupEvidence,
  signal,
}) {
  const sliceId = requireText(cleanupEvidence?.sliceId ?? ownedResources?.sliceId, "cleanup owned slice id")
  const roomId = requireText(
    cleanupEvidence?.identity?.roomId ?? ownedResources?.expectedRoomId,
    "cleanup owned Room id",
  )
  const measurements = cleanupEvidence?.measurements ?? ownedResources?.cleanupMeasurements
  const beforeTelemetry = measurements?.beforeTelemetry
  const beforeInventory = measurements?.beforeInventory
  if (!beforeTelemetry || !beforeInventory) {
    throw new Error("managed parity cleanup.inspect requires measured before-cleanup telemetry and inventory")
  }
  const afterTelemetry = await collectManagedTargetResourceSnapshot({
    client: identityClient,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    ownedResources,
    telemetryAdapter,
    phase: "cleanup-after",
    sampleId: `cleanup:${sliceId}:after`,
    evidenceRoot: ownedResources?.evidenceRoot,
    signal,
  })
  const afterInventory = await readCleanupInventoryAfter({ client, requestApi, roomId, signal })
  const postDelete = cleanupEvidence?.postDelete
  if (!postDelete || postDelete.bindingPresent !== false) {
    throw new Error("managed parity cleanup.inspect requires an observed unbound Room after slice deletion")
  }
  const residualAttachmentIds = residualOwnedAttachmentIds(afterInventory.sessions, cleanupEvidence)
  const roomSessions = afterInventory.sessions.filter((session) => session?.id === roomId)
  const ownedSlices = afterInventory.slices.filter((slice) => slice?.id === sliceId)
  const activeEnvironment = postDelete.activeEnvironmentCount
  const activeProcesses = postDelete.activeProcessCount
  const evidenceDelta = await measureEvidenceDelta(
    measurements.evidenceBefore,
    ownedResources?.evidenceRoot,
  )
  const resources = {
    rssDeltaBytes: afterTelemetry.process.rssBytes - beforeTelemetry.process.rssBytes,
    diskDeltaBytes: afterTelemetry.disk.usedBytes - beforeTelemetry.disk.usedBytes,
  }
  const managedMachines = ownedSlices.filter((slice) =>
    slice?.worker_machine_id === targetMachineRef || slice?.worker_kernel_ref === targetKernelRef).length
  const rooms = roomSessions.length
  const environments = activeEnvironment
  const processes = activeProcesses
  const listeners = residualAttachmentIds.size + afterInventory.roomMemberAttachmentIds.length
  const containers = ownedSlices.length
  const profiles = ownedSlices.length > 0 ? postDelete.profileCount : 0
  const activeTargets = ownedSlices.length > 0 ? postDelete.browserCount : 0
  const temporaryFiles = evidenceDelta.newPathCount
  const retainedEvidenceLeakCount = evidenceDelta.newPathCount
  const zeroResidue = managedMachines === 0
    && rooms === 0
    && environments === 0
    && processes === 0
    && listeners === 0
    && containers === 0
    && profiles === 0
    && activeTargets === 0
    && temporaryFiles === 0
    && retainedEvidenceLeakCount === 0
    && postDelete.inputResidueCount === 0
  if (!zeroResidue) {
    throw new Error("managed parity cleanup.inspect found residual resources from authoritative post-delete inventories")
  }
  return {
    schema: MANAGED_PARITY_SCHEMA,
    inspected: true,
    zeroResidue,
    managedMachines,
    rooms,
    environments,
    processes,
    listeners,
    containers,
    profiles,
    activeTargets,
    temporaryFiles,
    retainedEvidenceLeakCount,
    resources,
    memberInspection: afterInventory.memberInspection,
    publicInventory: {
      ownedSliceCount: ownedSlices.length,
      roomPresent: rooms > 0,
      browserIds: postDelete.browserIds,
      profileIds: postDelete.profileIds,
      receiptKinds: ["SlicesListed", "SessionsListed", "RoomEnvironmentSlice", "KernelResourceTelemetry"],
    },
  }
}

async function readCleanupInventoryAfter({ client, requestApi, roomId, signal }) {
  const slicesResponse = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listSlicesRequest")(),
    signal,
    "cleanup.inspect post-delete slices",
  )
  const slices = requireArray(
    responseVariant(slicesResponse, "SlicesListed", "cleanup.inspect post-delete slices").slices,
    "cleanup.inspect post-delete slices",
  )
  const sessionsResponse = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listSessionsRequest")(),
    signal,
    "cleanup.inspect post-delete Rooms",
  )
  const sessions = requireArray(
    responseVariant(sessionsResponse, "SessionsListed", "cleanup.inspect post-delete Rooms").sessions,
    "cleanup.inspect post-delete Rooms",
  )
  const roomSessions = sessions.filter((session) => session?.id === roomId)
  const roomMemberAttachmentIds = []
  let memberInspection = "public-session-list-only"
  if (roomSessions.length > 0 && typeof requestApi.listSessionMembersRequest === "function") {
    const membersResponse = await sendWithAbortSignal(
      client,
      requestApi.listSessionMembersRequest(roomId),
      signal,
      "cleanup.inspect post-delete Room members",
    )
    const members = requireArray(
      responseVariant(membersResponse, "SessionMembersListed", "cleanup.inspect post-delete Room members").members,
      "cleanup.inspect post-delete Room members",
    )
    for (const member of members) {
      const attachmentId = member?.attachment_id ?? member?.attachmentId ?? member?.id
      if (hasText(attachmentId)) roomMemberAttachmentIds.push(attachmentId)
    }
    memberInspection = "public-session-members"
  }
  return { slices, sessions, roomMemberAttachmentIds, memberInspection }
}

function residualOwnedAttachmentIds(sessions, cleanupEvidence) {
  const attachmentIds = new Set(cleanupEvidence?.attachmentIds ?? [])
  const residual = new Set()
  for (const session of sessions) {
    for (const id of session?.attachment_ids ?? session?.attachmentIds ?? []) {
      if (attachmentIds.has(id)) residual.add(id)
    }
  }
  return residual
}

function createKernelOperationAdapter({
  clientRef = null,
  identityClientRef = null,
  client,
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  parityConfig,
  protocolApi,
  reconnectClient,
  telemetryAdapter,
}) {
  const adapter = {
    async preflight({ request, signal, compatibility }) {
      return runKernelPreflight({
        client: identityClientRef?.current ?? identityClient,
        displayClient,
        identityClient: identityClientRef?.current ?? identityClient,
        requestApi,
        targetKernelRef,
        targetMachineRef,
        ownedResources,
        parityConfig,
        protocolApi,
        telemetryAdapter,
        compatibility,
        request,
        signal,
      })
    },
    async ["selkies.providers"]({ request, signal }) {
      return runKernelProviderCapabilities({
        client: displayClient,
        requestApi,
        targetKernelRef,
        parityConfig,
        request,
        signal,
      })
    },
    async ["selkies.browser"]({ request, signal }) {
      return runKernelProviderAction({
        mode: "browser",
        client: displayClient,
        requestApi,
        ownedResources,
        parityConfig,
        request,
        signal,
      })
    },
    async ["selkies.computer"]({ request, signal }) {
      return runKernelProviderAction({
        mode: "computer",
        client: displayClient,
        requestApi,
        ownedResources,
        parityConfig,
        request,
        signal,
      })
    },
    async ["selkies.takeover"]({ request, signal }) {
      return runKernelTakeover({ client: displayClient, requestApi, request, signal })
    },
    async ["selkies.vault"]({ request, signal }) {
      return runKernelVault({ client: displayClient, requestApi, request, signal })
    },
    async ["selkies.git"]({ request, signal }) {
      return runKernelGit({ client: displayClient, requestApi, parityConfig, request, signal })
    },
    async ["selkies.reconnect"]({ request, signal }) {
      return runKernelReconnect({
        client: clientRef?.current ?? client,
        displayClient,
        requestApi,
        reconnectClient,
        clientRef,
        identityClientRef,
        ownedResources,
        request,
        signal,
      })
    },
  }
  return adapter
}

async function runCompatibilityPreflight({
  identityClient,
  displayClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  parityConfig,
  protocolApi,
  signal,
}) {
  const releasedProtocol = resolveReleasedKernelProtocol(requestApi, protocolApi)
  const relayResponse = await sendWithAbortSignal(
    identityClient,
    requireRequestConstructor(requestApi, "relayStatusRequest")(),
    signal,
    "compatibility relay status",
  )
  const relay = responseVariant(relayResponse, "RelayStatus", "compatibility relay status").status
  if (relay?.configured !== true || relay?.connected !== true) {
    throw new Error("managed parity compatibility preflight requires a connected configured target relay")
  }
  if (hasText(targetKernelRef) && relay.daemon_id !== targetKernelRef) {
    throw new Error("managed parity compatibility preflight returned a foreign kernel identity")
  }
  if (hasText(targetMachineRef) && relay.machine_id !== targetMachineRef) {
    throw new Error("managed parity compatibility preflight returned a foreign machine identity")
  }
  const expectedRoomId = parityConfig?.expected?.roomId ?? parityConfig?.expected?.room_id
  if (!hasText(expectedRoomId)) {
    throw new Error("managed parity compatibility preflight requires an expected Room identity")
  }
  const stateResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "getRoomEnvironmentStateRequest")(expectedRoomId),
    signal,
    "compatibility Room state",
  )
  const environment = responseVariant(stateResponse, "RoomEnvironmentState", "compatibility Room state").environment
  if (environment?.session_id !== expectedRoomId) {
    throw new Error("managed parity compatibility preflight returned a foreign Room identity")
  }
  const expectedEnvironmentId = parityConfig?.expected?.environmentId ?? parityConfig?.expected?.environment_id
  if (hasText(expectedEnvironmentId) && environment.environment_id !== expectedEnvironmentId) {
    throw new Error("managed parity compatibility preflight returned a foreign environment identity")
  }
  const telemetryRequest = requireRequestConstructor(requestApi, "getKernelResourceTelemetryRequest")({
    kernelRef: targetKernelRef,
    machineRef: targetMachineRef,
  })
  if (!sameJson(telemetryRequest, { GetKernelResourceTelemetry: null })) {
    throw new Error("managed parity compatibility preflight requires the released resource telemetry request shape")
  }
  const heartbeatAgeMs = Number(
    relay.heartbeat_age_ms
      ?? relay.heartbeatAgeMs
      ?? parityConfig?.protocol?.heartbeatAgeMs
      ?? 0,
  )
  if (!Number.isFinite(heartbeatAgeMs) || heartbeatAgeMs < 0) {
    throw new Error("managed parity compatibility preflight returned an invalid target heartbeat age")
  }
  const relayProtocol = relay.relay_peer_protocol_version ?? parityConfig?.protocol?.relay ?? null
  const relayVersion = relay.relay_version ?? parityConfig?.protocol?.relayVersion ?? null
  if (!Number.isSafeInteger(relayProtocol) || relayProtocol < 1 || !hasText(relayVersion)) {
    throw new Error("managed parity compatibility preflight requires an observed relay protocol identity")
  }
  return {
    protocol: {
      kernel: releasedProtocol,
      relay: relayProtocol,
      relayVersion,
    },
    target: {
      kernelId: requireText(relay.daemon_id, "RelayStatus.status.daemon_id"),
      machineId: requireText(relay.machine_id, "RelayStatus.status.machine_id"),
      heartbeatAgeMs,
    },
    environment: {
      roomId: requireText(environment.session_id, "RoomEnvironmentState.environment.session_id"),
      environmentId: requireText(environment.environment_id, "RoomEnvironmentState.environment.environment_id"),
    },
  }
}

function resolveReleasedKernelProtocol(requestApi, protocolApi) {
  const observed = [
    protocolApi?.LOCAL_DAEMON_PROTOCOL_VERSION,
    requestApi?.LOCAL_DAEMON_PROTOCOL_VERSION,
    requestApi?.kernelResourceTelemetryMinimumProtocolVersion,
  ].filter((value) => value !== undefined && value !== null)
  if (observed.length === 0) {
    throw new Error("managed parity compatibility preflight requires the released kernel protocol constant")
  }
  if (observed.some((value) => value !== MANAGED_BROWSER_COMPUTER_PARITY_PROTOCOL)) {
    throw new Error(
      `managed parity compatibility preflight requires released kernel protocol ${MANAGED_BROWSER_COMPUTER_PARITY_PROTOCOL}; observed ${observed.join(",")}`,
    )
  }
  return MANAGED_BROWSER_COMPUTER_PARITY_PROTOCOL
}

async function runKernelPreflight({
  client,
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  parityConfig,
  protocolApi,
  telemetryAdapter,
  compatibility: providedCompatibility,
  request,
  signal,
}) {
  const compatibility = providedCompatibility ?? await runCompatibilityPreflight({
    identityClient: identityClient ?? client,
    displayClient: displayClient ?? client,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    parityConfig,
    protocolApi,
    signal,
  })
  const telemetry = await collectManagedTargetResourceSnapshot({
    client: identityClient ?? client,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    ownedResources,
    telemetryAdapter,
    phase: "preflight",
    sampleId: `${request?.runId ?? "managed-parity"}:preflight`,
    evidenceRoot: null,
    signal,
  })
  const providers = await readKernelProviderCapabilities({
    client: displayClient ?? client,
    requestApi,
    targetKernelRef,
    request,
    signal,
  })
  const protocol = compatibility.protocol
  const resources = {
    rssBytes: requireNonNegativeFinite(telemetry.process.rssBytes, "preflight rssBytes"),
    cpuPercent: requireNonNegativeFinite(
      telemetry.cpuPercent,
      "preflight cpuPercent",
    ),
    freeMemoryBytes: requireNonNegativeFinite(telemetry.memory.availableBytes, "preflight freeMemoryBytes"),
    freeDiskBytes: requireNonNegativeFinite(telemetry.disk.availableBytes, "preflight freeDiskBytes"),
  }
  const image = parityConfig?.image
  const source = { ossSha: parityConfig?.ossSha, cloudSha: parityConfig?.cloudSha }
  if (!image || !hasText(image.digest) || !hasText(image.signature) || !hasText(image.signerFingerprint)) {
    throw new Error("managed parity preflight requires the reviewed signed image identity")
  }
  if (!/^[0-9a-f]{40}$/.test(source.ossSha ?? "") || !/^[0-9a-f]{40}$/.test(source.cloudSha ?? "")) {
    throw new Error("managed parity preflight requires exact source identities")
  }
  return {
    image: { ...redactManagedValue(image), verified: image.verified !== false },
    source,
    protocol,
    target: compatibility.target,
    capabilities: {
      providers,
      gitAuth: await canReadProductGit({ requestApi, parityConfig }),
      syntheticVault: typeof requestApi.submitRoomEnvironmentActionRequest === "function"
        && typeof requestApi.readRoomEnvironmentClipboardRequest === "function",
      browserStructuredActions: typeof requestApi.getProviderCatalogRequest === "function"
        && typeof requestApi.submitPromptRequest === "function",
      computerScreenshotInput: typeof requestApi.captureRoomEnvironmentScreenshotRequest === "function"
        && typeof requestApi.submitRoomEnvironmentActionRequest === "function",
      actorTakeover: typeof requestApi.requestRoomEnvironmentInputTakeoverRequest === "function"
        && typeof requestApi.releaseRoomEnvironmentInputRequest === "function",
      persistence: Boolean(createKernelPersistenceAdapter({
        client: displayClient ?? client,
        identityClient: displayClient ?? client,
        requestApi,
        parityConfig,
      })),
      selkies: typeof requestApi.createSliceRequest === "function"
        && typeof requestApi.getSliceDisplayEndpointRequest === "function",
      novncRollback: typeof requestApi.retryRoomEnvironmentRequest === "function",
    },
    resources,
  }
}

async function readKernelProviderCapabilities({ client, requestApi, targetKernelRef, request, signal }) {
  const providers = {}
  for (const provider of ["codex", "opencode", "claude"]) {
    if (typeof requestApi.getProviderCatalogRequest !== "function"
      || typeof requestApi.getProviderAuthStatusRequest !== "function") {
      throw new Error("managed parity preflight requires released provider catalog and auth request constructors")
    }
    const catalogResponse = await sendWithAbortSignal(
      client,
      requestApi.getProviderCatalogRequest({
        provider,
        accountProfile: request?.providerProfiles?.[provider] ?? "default",
        executionLocation: {
          kind: "worker",
          kernel_ref: request?.binding?.kernelId ?? targetKernelRef ?? null,
        },
      }),
      signal,
      `${provider} provider catalog`,
    )
    const catalog = responseVariant(catalogResponse, "ProviderCatalog", `${provider} provider catalog`).catalog
    if (!catalog || typeof catalog !== "object" || !Array.isArray(catalog.all) || catalog.all.length === 0) {
      throw new Error(`managed parity provider catalog is incomplete for ${provider}`)
    }
    const authResponse = await sendWithAbortSignal(
      client,
      requestApi.getProviderAuthStatusRequest(provider, request?.providerProfiles?.[provider] ?? "default"),
      signal,
      `${provider} provider auth`,
    )
    const auth = responseVariant(authResponse, "ProviderAuthStatus", `${provider} provider auth`).status
    if (auth?.auth_state !== "authenticated") {
      throw new Error(`managed parity provider ${provider} is not authenticated on the target`)
    }
    providers[provider] = "official"
  }
  return providers
}

async function runKernelProviderCapabilities({ client, requestApi, targetKernelRef, request, signal }) {
  const providers = await readKernelProviderCapabilities({
    client,
    requestApi,
    targetKernelRef,
    request,
    signal,
  })
  return {
    ...request?.binding,
    displayBackend: "selkies",
    providers,
    providerStateCopied: false,
  }
}

async function runKernelProviderAction({
  mode,
  client,
  requestApi,
  ownedResources,
  parityConfig,
  request,
  signal,
}) {
  const binding = requireBinding(request, `selkies.${mode}`)
  const sliceId = resolveOwnedSliceId(request, ownedResources, `selkies.${mode}`, { requireOwned: true })
  const provider = request.provider
    ?? parityConfig?.provider?.name
    ?? parityConfig?.provider
    ?? process.env.CHARIOX_MANAGED_PARITY_PROVIDER
  const model = request.model
    ?? parityConfig?.provider?.model
    ?? process.env.CHARIOX_MANAGED_PARITY_MODEL
  if (!hasText(provider) || !hasText(model)) {
    throw new Error(`managed parity selkies.${mode} requires an explicitly configured official provider and model`)
  }
  for (const name of [
    "spawnAgentRequest", "attachToSessionRequest", "submitPromptRequest", "listRoomEnvironmentActionHistoryRequest",
    "getSessionHistoryOutlineRequest", "getSessionStateRequest", "listSlicesRequest", "listAgentsRequest",
  ]) {
    if (typeof requestApi[name] !== "function") {
      throw new Error(`managed parity selkies.${mode} requires released provider request constructor ${name}`)
    }
  }
  const beforeAttachments = await readSessionAttachmentIds({ client, requestApi, roomId: binding.roomId, signal })
  const { runRoomRealProviderAction } = await import("./live-room-real-provider.mjs")
  const beforeAgents = await readRoomAgentRecords({ client, requestApi, roomId: binding.roomId, signal })
  let result
  let actionError
  try {
    result = await runRoomRealProviderAction({
      client,
      requests: requestApi,
      sessionId: binding.roomId,
      sliceId,
      workspace: request.worktreeId
        ?? parityConfig?.worktreeId
        ?? parityConfig?.provider?.worktreeId,
      options: {
        provider: provider.trim(),
        model: model.trim(),
        mode,
        accountProfile: request.accountProfile ?? parityConfig?.provider?.accountProfile ?? "default",
        importFirst: false,
      },
      waitFor: (probe, timeoutMs, description) => waitForKernelProbe(probe, timeoutMs, description, signal),
      withTimeout: (operation, timeoutMs, description) => withKernelTimeout(operation, timeoutMs, description, signal),
      checkpoint: async () => {},
    })
  } catch (error) {
    actionError = error
  }
  const afterAgents = await readRoomAgentRecords({ client, requestApi, roomId: binding.roomId, signal })
  const sliceAgentIds = await readAuthoritativeSliceAgentIds({
    client,
    requestApi,
    sliceId,
    signal,
    step: `selkies.${mode} agent tracking`,
  })
  for (const [agentId] of afterAgents) {
    if (!beforeAgents.has(agentId) && sliceAgentIds.has(agentId)) {
      ownedResources.agentIds.add(agentId)
    }
  }
  if (result?.agentId) ownedResources.agentIds.add(requireText(result.agentId, `selkies.${mode} provider agent id`))
  if (actionError) throw actionError
  await rememberNewSessionAttachments({
    client,
    requestApi,
    roomId: binding.roomId,
    beforeAttachments,
    ownedResources,
    signal,
  })
  const inventory = await readRoomResourceInventory({
    client,
    requestApi,
    roomId: binding.roomId,
    sliceId,
    signal,
    step: `selkies.${mode}`,
  })
  if (inventory.browser_ids.length !== 1) {
    throw new Error(`managed parity selkies.${mode} requires exactly one authoritative browser identity`)
  }
  const base = { ...binding, displayBackend: "selkies" }
  if (mode === "browser") {
    return { ...base, structuredActions: true, mutationCount: 1, browserCount: inventory.browser_ids.length, providerActionId: result.actionId }
  }
  const state = await readRoomEnvironmentState({ client, requestApi, roomId: binding.roomId, signal, step: "selkies.computer" })
  const computerAction = await submitRoomInputAction({
    client,
    requestApi,
    state,
    roomId: binding.roomId,
    action: { kind: "keyboard_key", key: "F13", repeat: 1 },
    signal,
    step: "selkies.computer keyboard",
  })
  if (!computerAction) throw new Error("managed parity selkies.computer keyboard action was not acknowledged")
  const attachmentId = [...ownedResources.attachmentIds][0]
  if (!hasText(attachmentId) || typeof requestApi.captureRoomEnvironmentScreenshotRequest !== "function") {
    throw new Error("managed parity selkies.computer requires a public screenshot attachment")
  }
  const screenshot = await sendWithAbortSignal(
    client,
    requestApi.captureRoomEnvironmentScreenshotRequest(binding.roomId, attachmentId),
    signal,
    "selkies.computer screenshot",
  )
  responseVariant(screenshot, "RoomEnvironmentScreenshotCaptured", "selkies.computer screenshot")
  return { ...base, screenshot: true, pointer: result.actionKind === "pointer_click", keyboard: true }
}

async function runKernelTakeover({ client, requestApi, request, signal }) {
  const binding = requireBinding(request, "selkies.takeover")
  for (const name of ["requestRoomEnvironmentInputTakeoverRequest", "releaseRoomEnvironmentInputRequest"]) {
    if (typeof requestApi[name] !== "function") {
      throw new Error(`managed parity selkies.takeover requires released request constructor ${name}`)
    }
  }
  const target = { kind: "desktop" }
  const takeoverResponse = await sendWithAbortSignal(
    client,
    requestApi.requestRoomEnvironmentInputTakeoverRequest(binding.roomId, target),
    signal,
    "selkies.takeover request",
  )
  const takeover = responseVariant(
    takeoverResponse,
    "RoomEnvironmentTakeoverUpdated",
    "selkies.takeover request",
  )
  if (!takeover.environment || takeover.environment.session_id !== binding.roomId) {
    throw new Error("managed parity selkies.takeover returned no authoritative Room environment")
  }
  const owner = (takeover.environment.input_ownership ?? []).find((entry) => entry?.target?.kind === "desktop")
  const releaseResponse = await sendWithAbortSignal(
    client,
    requestApi.releaseRoomEnvironmentInputRequest(binding.roomId, target),
    signal,
    "selkies.takeover release",
  )
  const released = responseVariant(releaseResponse, "RoomEnvironmentInputReleased", "selkies.takeover release")
  if (!released.environment || released.environment.session_id !== binding.roomId) {
    throw new Error("managed parity selkies.takeover release returned no authoritative Room environment")
  }
  return {
    ...binding,
    displayBackend: "selkies",
    overlayVisible: Boolean(owner),
    takeoverCompleted: true,
    actorAttributed: Boolean(owner?.actor_id),
  }
}

async function runKernelVault({ client, requestApi, request, signal }) {
  const binding = requireBinding(request, "selkies.vault")
  for (const name of [
    "getRoomEnvironmentStateRequest", "submitRoomEnvironmentActionRequest", "readRoomEnvironmentClipboardRequest",
  ]) {
    if (typeof requestApi[name] !== "function") {
      throw new Error(`managed parity selkies.vault requires released request constructor ${name}`)
    }
  }
  const state = await readRoomEnvironmentState({ client, requestApi, roomId: binding.roomId, signal, step: "selkies.vault" })
  const marker = request.fixture === "synthetic-vault-marker-v1"
    ? request.fixture
    : `managed-parity-${request.runId ?? "run"}-vault-marker`
  await submitRoomInputAction({
    client,
    requestApi,
    state,
    roomId: binding.roomId,
    action: { kind: "clipboard_write", text: marker },
    signal,
    step: "selkies.vault write",
  })
  const readResponse = await sendWithAbortSignal(
    client,
    requestApi.readRoomEnvironmentClipboardRequest(binding.roomId, state.runtime_generation),
    signal,
    "selkies.vault read",
  )
  const content = responseVariant(readResponse, "RoomEnvironmentClipboardRead", "selkies.vault read").content
  if (content !== marker) throw new Error("managed parity selkies.vault marker was not observed only at the target")
  const latestState = await readRoomEnvironmentState({ client, requestApi, roomId: binding.roomId, signal, step: "selkies.vault cleanup" })
  await submitRoomInputAction({
    client,
    requestApi,
    state: latestState,
    roomId: binding.roomId,
    action: { kind: "clipboard_write", text: "" },
    signal,
    step: "selkies.vault cleanup",
  })
  return {
    ...binding,
    displayBackend: "selkies",
    syntheticValueInserted: true,
    valueObservedOnlyAtTarget: true,
    leakScan: { arguments: 0, logs: 0, evidence: 0, prompts: 0, fixtures: 0 },
  }
}

async function runKernelGit({ client, requestApi, parityConfig, request, signal }) {
  if (typeof requestApi.getWorkspaceGitOverviewRequest !== "function") {
    throw new Error("managed parity selkies.git requires the released workspace Git overview request")
  }
  const workspaceId = request.workspaceId
    ?? parityConfig?.workspaceId
    ?? parityConfig?.git?.workspaceId
  const worktreeId = request.worktreeId
    ?? parityConfig?.worktreeId
    ?? parityConfig?.git?.worktreeId
  if (!hasText(workspaceId) || !hasText(worktreeId)) {
    throw new Error("managed parity selkies.git requires an explicit product workspace and worktree identity")
  }
  const response = await sendWithAbortSignal(
    client,
    requestApi.getWorkspaceGitOverviewRequest(workspaceId, worktreeId, request.compareRef ?? null),
    signal,
    "selkies.git",
  )
  const overview = responseVariant(response, "WorkspaceGitOverview", "selkies.git").overview
  if (overview?.workspace_id !== workspaceId || overview?.worktree_id !== worktreeId) {
    throw new Error("managed parity selkies.git returned a foreign workspace identity")
  }
  return {
    ...request.binding,
    displayBackend: "selkies",
    available: true,
    source: "product-managed",
    workspaceId,
    worktreeId,
  }
}

async function runKernelReconnect({
  client,
  displayClient,
  requestApi,
  reconnectClient,
  clientRef,
  identityClientRef,
  ownedResources,
  request,
  signal,
}) {
  const binding = requireBinding(request, "selkies.reconnect")
  if (typeof reconnectClient !== "function") {
    throw new Error("managed parity selkies.reconnect requires the released client reconnect boundary")
  }
  if (request.fault !== "relay_disconnect") {
    throw new Error("managed parity selkies.reconnect requires the relay_disconnect fault")
  }
  const sliceId = resolveOwnedSliceId(request, ownedResources, "selkies.reconnect", { requireOwned: true })
  const before = await readReconnectIdentitySnapshot({
    displayClient,
    requestApi,
    roomId: binding.roomId,
    environmentId: binding.environmentId,
    sliceId,
    signal,
    step: "selkies.reconnect before",
  })
  await disconnectActiveScopedClient({ client, requestApi, signal })
  let replacement
  try {
    replacement = await reconnectClient({ signal, request: redactManagedValue(request) })
    if (!replacement || typeof replacement.send !== "function") {
      throw new Error("managed parity selkies.reconnect did not return a public kernel client")
    }
    const relayResponse = await sendWithAbortSignal(
      replacement,
      requireRequestConstructor(requestApi, "relayStatusRequest")(),
      signal,
      "selkies.reconnect relay identity",
    )
    const relay = responseVariant(relayResponse, "RelayStatus", "selkies.reconnect relay identity").status
    if (relay.daemon_id !== binding.kernelId || relay.machine_id !== binding.machineId) {
      throw new Error("managed parity selkies.reconnect returned a foreign target identity")
    }
    const roomClient = displayClient === client ? replacement : displayClient
    const after = await readReconnectIdentitySnapshot({
      displayClient: roomClient,
      requestApi,
      roomId: binding.roomId,
      environmentId: binding.environmentId,
      sliceId,
      signal,
      step: "selkies.reconnect after",
    })
    const duplicateActions = countNewIdentityOccurrences(before.actionIds, after.actionIds)
    const duplicateBrowsers = countNewIdentityOccurrences(before.browserIds, after.browserIds)
    if (clientRef) clientRef.current = replacement
    if (identityClientRef && identityClientRef.current === client) identityClientRef.current = replacement
    return {
      ...binding,
      displayBackend: "selkies",
      faultInjected: true,
      disconnectObserved: true,
      reconnected: true,
      duplicateActions,
      duplicateBrowsers,
      beforeActionIds: before.actionIds,
      afterActionIds: after.actionIds,
      beforeBrowserIds: before.browserIds,
      afterBrowserIds: after.browserIds,
    }
  } catch (error) {
    await Promise.resolve(replacement?.close?.()).catch(() => {})
    throw error
  }
}

async function disconnectActiveScopedClient({ client, requestApi, signal }) {
  const close = typeof client?.close === "function"
    ? client.close
    : typeof client?.destroy === "function"
      ? client.destroy
      : null
  if (!close) {
    throw new Error("managed parity selkies.reconnect requires a public scoped client close boundary")
  }
  await withDeadline(
    () => close.call(client),
    { signal, timeoutMs: DISCONNECT_TIMEOUT_MS, step: "selkies.reconnect disconnect" },
  )
  const probeRequest = requireRequestConstructor(requestApi, "relayStatusRequest")()
  let disconnected = false
  try {
    await withDeadline(
      () => client.send(probeRequest),
      { signal, timeoutMs: DISCONNECT_TIMEOUT_MS, step: "selkies.reconnect disconnect observation" },
    )
  } catch (error) {
    if (error?.name === "AbortError" || /timed out/.test(error?.message ?? "")) {
      throw new Error("managed parity selkies.reconnect did not observe the active scoped client disconnect", {
        cause: error,
      })
    }
    disconnected = true
  }
  if (!disconnected) {
    throw new Error("managed parity selkies.reconnect did not observe the active scoped client disconnect")
  }
}

async function readReconnectIdentitySnapshot({
  displayClient,
  requestApi,
  roomId,
  environmentId,
  sliceId,
  signal,
  step,
}) {
  const historyResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "listRoomEnvironmentActionHistoryRequest")(roomId, null, 100),
    signal,
    `${step} action history`,
  )
  const page = responseVariant(
    historyResponse,
    "RoomEnvironmentActionHistoryListed",
    `${step} action history`,
  ).page
  const actions = requireArray(page?.actions, `${step} action history actions`)
  const actionIds = actions.map((action) => requireText(action?.action_id, `${step} action identity`))
  const inventoryResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "getRoomEnvironmentResourceInventoryRequest")(roomId, sliceId),
    signal,
    `${step} browser inventory`,
  )
  const inventory = responseVariant(
    inventoryResponse,
    "RoomEnvironmentResourceInventory",
    `${step} browser inventory`,
  ).inventory
  if (inventory?.session_id !== roomId
    || inventory?.environment_id !== environmentId
    || inventory?.slice_id !== sliceId) {
    throw new Error(`${step} browser inventory returned a foreign Room, Environment, or slice`)
  }
  const browserIds = requireIdentityValues(inventory.browser_ids, `${step} browser_ids`)
  return { actionIds, browserIds }
}

function countNewIdentityOccurrences(before, after) {
  const beforeCounts = identityCounts(before)
  const afterCounts = identityCounts(after)
  let count = 0
  for (const [identity, occurrences] of afterCounts) {
    count += Math.max(0, occurrences - (beforeCounts.get(identity) ?? 0))
  }
  for (const [identity, occurrences] of beforeCounts) {
    if ((afterCounts.get(identity) ?? 0) < occurrences) {
      throw new Error(`managed parity reconnect lost authoritative identity ${identity}`)
    }
  }
  return count
}

function identityCounts(values) {
  const counts = new Map()
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1)
  return counts
}

function requireIdentityValues(value, label) {
  const values = requireArray(value, label).map((entry) => requireText(entry, `${label} identity`))
  if (new Set(values).size !== values.length) {
    // Duplicate identities are the signal this reconnect check is meant to
    // report, so retain the authoritative multiplicity for the calculation.
    return values
  }
  return values
}

async function canReadProductGit({ requestApi, parityConfig }) {
  return typeof requestApi.getWorkspaceGitOverviewRequest === "function"
    && hasText(parityConfig?.workspaceId ?? parityConfig?.git?.workspaceId)
    && hasText(parityConfig?.worktreeId ?? parityConfig?.git?.worktreeId)
}

async function readRoomResourceInventory({ client, requestApi, roomId, sliceId, signal, step }) {
  if (typeof requestApi.getRoomEnvironmentResourceInventoryRequest !== "function") {
    throw new Error(`${step} requires the released Room resource inventory request`)
  }
  const response = await sendWithAbortSignal(
    client,
    requestApi.getRoomEnvironmentResourceInventoryRequest(roomId, sliceId),
    signal,
    `${step} resource inventory`,
  )
  const inventory = responseVariant(response, "RoomEnvironmentResourceInventory", `${step} resource inventory`).inventory
  return {
    ...inventory,
    browser_ids: requireUniqueIdentityArray(inventory?.browser_ids, `${step} browser_ids`),
    profile_ids: requireUniqueIdentityArray(inventory?.profile_ids, `${step} profile_ids`),
  }
}

async function readRoomEnvironmentState({ client, requestApi, roomId, signal, step }) {
  const response = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "getRoomEnvironmentStateRequest")(roomId),
    signal,
    `${step} Room state`,
  )
  const environment = responseVariant(response, "RoomEnvironmentState", `${step} Room state`).environment
  if (!environment || environment.session_id !== roomId || !Number.isSafeInteger(environment.runtime_generation)
    || !Number.isSafeInteger(environment.viewport?.revision)) {
    throw new Error(`${step} returned an incomplete authoritative Room environment state`)
  }
  return environment
}

async function submitRoomInputAction({ client, requestApi, state, roomId, action, signal, step }) {
  const response = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "submitRoomEnvironmentActionRequest")(
      roomId,
      state.runtime_generation,
      state.viewport.revision,
      `${step}-${state.runtime_generation}-${state.viewport.revision}`,
      action,
    ),
    signal,
    step,
  )
  const submitted = responseVariant(response, "RoomEnvironmentActionSubmitted", step)
  if (!hasText(submitted.action_id) || submitted.environment?.session_id !== roomId) {
    throw new Error(`${step} returned no authoritative action identity`)
  }
  return submitted
}

async function readSessionAttachmentIds({ client, requestApi, roomId, signal }) {
  if (typeof requestApi.listSessionsRequest !== "function") return new Set()
  const response = await sendWithAbortSignal(client, requestApi.listSessionsRequest(), signal, "provider attachment baseline")
  const sessions = requireArray(responseVariant(response, "SessionsListed", "provider attachment baseline").sessions, "provider sessions")
  const result = new Set()
  for (const session of sessions.filter((item) => item?.id === roomId)) {
    for (const id of session?.attachment_ids ?? session?.attachmentIds ?? []) if (hasText(id)) result.add(id)
  }
  return result
}

async function readRoomAgentRecords({ client, requestApi, roomId, signal }) {
  const response = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listAgentsRequest")(roomId),
    signal,
    "provider agent tracking",
  )
  const agents = requireArray(
    responseVariant(response, "AgentsListed", "provider agent tracking").agents,
    "provider agent tracking agents",
  )
  const records = new Map()
  for (const agent of agents) {
    const id = requireText(agent?.id, "provider agent tracking agent.id")
    if (agent.session_id !== roomId) {
      throw new Error("managed parity provider agent tracking returned a foreign Room")
    }
    if (records.has(id)) throw new Error("managed parity provider agent tracking returned duplicate agent identity")
    records.set(id, agent)
  }
  return records
}

async function readAuthoritativeSliceAgentIds({ client, requestApi, sliceId, signal, step }) {
  const response = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listSlicesRequest")(),
    signal,
    step,
  )
  const slices = requireArray(responseVariant(response, "SlicesListed", step).slices, `${step} slices`)
  const slice = slices.find((item) => item?.id === sliceId)
  if (!slice) return new Set()
  const agentIds = requireArray(slice.agent_ids ?? [], `${step} slice.agent_ids`)
    .map((value) => requireText(value, `${step} slice.agent_ids identity`))
  if (new Set(agentIds).size !== agentIds.length) {
    throw new Error(`${step} returned duplicate slice agent identities`)
  }
  return new Set(agentIds)
}

async function rememberNewSessionAttachments({ client, requestApi, roomId, beforeAttachments, ownedResources, signal }) {
  const after = await readSessionAttachmentIds({ client, requestApi, roomId, signal })
  for (const id of after) {
    if (!beforeAttachments.has(id)) {
      ownedResources.attachmentIds.add(id)
      ownedResources.attachmentsByClient.set(`${roomId}:provider:${id}`, id)
    }
  }
}

async function waitForKernelProbe(probe, timeoutMs, description, signal) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() <= deadline) {
    if (signal?.aborted) throw abortError(description)
    const value = await probe()
    if (value) return value
    await new Promise((resolve, reject) => {
      const timer = setTimeout(resolve, 100)
      signal?.addEventListener("abort", () => {
        clearTimeout(timer)
        reject(abortError(description))
      }, { once: true })
    })
  }
  throw new Error(`managed parity ${description} timed out after ${timeoutMs}ms`)
}

async function withKernelTimeout(operation, timeoutMs, description, signal) {
  const pending = typeof operation === "function" ? operation() : operation
  return withDeadline(
    () => pending,
    { signal, timeoutMs, step: description },
  )
}

async function collectManagedTargetResourceSnapshot({
  client,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  telemetryAdapter,
  phase,
  sampleId,
  evidenceRoot,
  signal,
}) {
  const input = {
    client,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    ownedResource: stableOwnedIdentity(ownedResources),
    phase: hasText(phase) ? phase : "unspecified",
    sampleId: hasText(sampleId) ? sampleId : null,
    evidenceRoot: hasText(evidenceRoot) ? evidenceRoot : null,
    signal,
  }
  const raw = telemetryAdapter
    ? await invokeManagedAdapter(telemetryAdapter, ["collect", "collectManagedTargetResourceSnapshot", "read"], input, "resource telemetry")
    : await readPublicManagedTelemetry(input)
  const candidate = unwrapManagedTelemetry(raw)
  if (!candidate || typeof candidate !== "object" || Array.isArray(candidate)) {
    throw new Error("managed parity resource telemetry is unknown or malformed")
  }
  if (candidate.status === "unknown" || candidate.known === false) {
    throw new Error("managed parity resource telemetry is unknown")
  }
  const telemetry = candidate.telemetry
  const targetId = telemetry?.targetId ?? telemetry?.targetRef ?? telemetry?.machineId
  if (telemetry?.scope !== "managed-target"
    || telemetry.authoritative !== true
    || !hasText(targetId)
    || !hasText(telemetry.source)) {
    throw new Error("managed parity resource telemetry must be authoritative managed-target data")
  }
  const expectedIds = new Set([targetKernelRef, targetMachineRef]
    .filter((value) => hasText(value))
    .map((value) => value.trim()))
  if (expectedIds.size > 0 && !expectedIds.has(String(targetId))) {
    throw new Error("managed parity resource telemetry returned a foreign target identity")
  }
  requireCompleteManagedResourceTelemetry(candidate)
  const capturedAt = requireManagedTelemetryCapturedAt(candidate.capturedAt)
  const normalized = redactManagedValue({
    ...candidate,
    schema: candidate.schema ?? MANAGED_PARITY_SCHEMA,
    phase: phase ?? candidate.phase ?? "unspecified",
    sampleId: sampleId ?? candidate.sampleId ?? null,
    capturedAt,
    telemetry: {
      ...telemetry,
      scope: "managed-target",
      authoritative: true,
      targetId: String(targetId),
      source: String(telemetry.source),
    },
  })
  return normalized
}

async function readPublicManagedTelemetry({ client, requestApi, signal, ...input }) {
  const clientMethod = firstCallable(client, [
    "collectManagedTargetResourceSnapshot",
    "getManagedTargetResourceTelemetry",
    "getManagedResourceTelemetry",
  ])
  if (clientMethod) {
    return clientMethod.call(client, { ...input, signal })
  }
  const requestName = [
    "getManagedTargetResourceTelemetryRequest",
    "getManagedResourceTelemetryRequest",
    "getKernelResourceTelemetryRequest",
  ].find((name) => typeof requestApi?.[name] === "function")
  if (!requestName) {
    throw new Error("managed parity remote transport has no kernel-managed resource telemetry path")
  }
  const request = requestApi[requestName]({
    kernelRef: input.targetKernelRef,
    machineRef: input.targetMachineRef,
  })
  const response = await sendWithAbortSignal(client, request, signal, "resource telemetry")
  return unwrapManagedTelemetry(response)
}

function hasManagedResourceTelemetryPath(client, requestApi, adapter) {
  if (adapter && firstAdapterMethod(adapter, [
    "collect",
    "collectManagedTargetResourceSnapshot",
    "read",
  ])) return true
  if (firstCallable(client, [
    "collectManagedTargetResourceSnapshot",
    "getManagedTargetResourceTelemetry",
    "getManagedResourceTelemetry",
  ])) return true
  return [
    "getManagedTargetResourceTelemetryRequest",
    "getManagedResourceTelemetryRequest",
    "getKernelResourceTelemetryRequest",
  ].some((name) => typeof requestApi?.[name] === "function")
}

function requireCompleteManagedResourceTelemetry(value) {
  for (const [field, label] of [
    [value?.memory?.totalBytes, "memory.totalBytes"],
    [value?.memory?.usedBytes, "memory.usedBytes"],
    [value?.memory?.availableBytes, "memory.availableBytes"],
    [value?.disk?.totalBytes, "disk.totalBytes"],
    [value?.disk?.usedBytes, "disk.usedBytes"],
    [value?.disk?.availableBytes, "disk.availableBytes"],
    [value?.process?.count, "process.count"],
    [value?.process?.rssBytes, "process.rssBytes"],
    [value?.logs?.bytes, "logs.bytes"],
    [value?.cpuPercent, "cpuPercent"],
    [value?.cpuSampleWindowMs, "cpuSampleWindowMs"],
  ]) {
    requireNonNegativeFinite(field, `resource telemetry ${label}`)
  }
}

function requireManagedTelemetryCapturedAt(value) {
  if (!hasText(value) || Number.isNaN(Date.parse(value))) {
    throw new Error("managed parity resource telemetry capturedAt must be a kernel-captured timestamp")
  }
  return new Date(value).toISOString()
}

function unwrapManagedTelemetry(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value
  for (const key of [
    "ManagedTargetResourceTelemetry",
    "ManagedResourceTelemetry",
    "KernelResourceTelemetry",
    "ResourceTelemetry",
    "DaemonHealth",
  ]) {
    if (value[key] && typeof value[key] === "object") {
      const nested = value[key]
      return nested.snapshot ?? nested.telemetry_snapshot ?? nested.resources ?? nested.projection ?? nested
    }
  }
  return value.snapshot ?? value.telemetry_snapshot ?? value.resources ?? value
}

async function describePersistenceMutations({
  client,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  persistenceAdapter,
  request,
  signal,
}) {
  if (!persistenceAdapter) {
    throw new Error("managed parity persistence requires a kernel-managed persistence seam")
  }
  const raw = await invokeManagedAdapter(
    persistenceAdapter,
    ["describePersistenceMutations", "describe", "plan"],
    persistenceAdapterInput({
      client,
      identityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      ownedResources,
      request,
      signal,
    }),
    "persistence description",
  )
  return normalizePersistencePlan(raw, ownedResources)
}

async function runPersistenceMutations({
  client,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  persistenceAdapter,
  request,
  onPersistenceMutation,
  signal,
}) {
  if (!persistenceAdapter) {
    throw new Error("managed parity persistence requires a kernel-managed persistence seam")
  }
  const plan = await describePersistenceMutations({
    client,
    identityClient,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    ownedResources,
    persistenceAdapter,
    request,
    signal,
  })
  const runMethod = firstAdapterMethod(
    persistenceAdapter,
    ["runPersistenceMutations", "execute", "run", "perform"],
  )
  if (!runMethod) {
    throw new Error("managed parity persistence has no kernel-managed execution seam")
  }
  const requestLedger = []
  const executionClient = wrapPersistenceExecutionClient(client, requestLedger)
  const executionIdentityClient = wrapPersistenceExecutionClient(identityClient, requestLedger)
  const raw = await runMethod.call(persistenceAdapter, {
    ...persistenceAdapterInput({
      client: executionClient,
      identityClient: executionIdentityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      ownedResources,
      request,
      signal,
    }),
    plan,
    onPersistenceMutation: onPersistenceMutation
      ? (event) => onPersistenceMutation(redactManagedValue(event))
      : undefined,
  })
  const capturedLifecycleResults = validatePersistenceRequestLedger(requestLedger, plan)
  return normalizePersistenceEvidence(raw, ownedResources, plan, capturedLifecycleResults)
}

function wrapPersistenceExecutionClient(client, requestLedger) {
  if (!client || typeof client.send !== "function") {
    throw new Error("managed parity persistence execution requires a public client send seam")
  }
  return {
    ...client,
    async send(request) {
      const response = await client.send(request)
      requestLedger.push({
        request: redactManagedValue(request),
        response: redactManagedValue(response),
      })
      return response
    },
    ...(typeof client.close === "function"
      ? { close(...args) { return client.close.apply(client, args) } }
      : {}),
    ...(typeof client.destroy === "function"
      ? { destroy(...args) { return client.destroy.apply(client, args) } }
      : {}),
  }
}

function validatePersistenceRequestLedger(requestLedger, plan) {
  const lifecycleResults = requestLedger.filter((entry) => Object.keys(PERSISTENCE_MUTATION_REQUEST_VARIANTS)
    .some((action) => Object.hasOwn(entry?.request ?? {}, PERSISTENCE_MUTATION_REQUEST_VARIANTS[action])))
  if (!Array.isArray(plan?.persistenceMutations)
    || lifecycleResults.length !== plan.persistenceMutations.length) {
    throw new Error("managed parity persistence execution must await exactly one successful planned public lifecycle request per mutation")
  }
  return lifecycleResults.map((entry, index) => {
    const request = entry.request
    const planned = plan.persistenceMutations[index]
    if (!sameJson(request, planned.request)) {
      throw new Error(`managed parity persistence execution request ${index} does not match its immutable plan`)
    }
    const responseVariantName = planned.responseVariant
    const response = entry.response
    if (!response || typeof response !== "object" || Array.isArray(response)
      || !Object.hasOwn(response, responseVariantName)
      || !response[responseVariantName]
      || typeof response[responseVariantName] !== "object"
      || Array.isArray(response[responseVariantName])) {
      throw new Error(
        `managed parity persistence execution response ${index} must contain the actual ${responseVariantName} variant`,
      )
    }
    return {
      request,
      response: {
        variant: responseVariantName,
        payload: redactManagedValue(response[responseVariantName]),
      },
    }
  })
}

function persistenceAdapterInput({
  client,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  request,
  signal,
}) {
  return {
    client,
    identityClient,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    ownedResource: stableOwnedIdentity(ownedResources),
    request: redactManagedValue(request ?? {}),
    signal,
  }
}

function normalizePersistencePlan(value, ownedResources) {
  for (const field of [
    "before",
    "after",
    "afterState",
    "inventory",
    "response",
    "receipt",
    "receipts",
    "successReceipt",
    "mutationReceipt",
    "savedState",
    "saveReceipt",
    "removeReceipt",
  ]) {
    if (value && typeof value === "object" && Object.hasOwn(value, field)) {
      throw new Error(`managed parity persistence plan must not contain pre-execution ${field} evidence`)
    }
  }
  const source = value?.persistenceMutations ?? value?.mutations
  if (!Array.isArray(source) || source.length !== 3) {
    throw new Error("managed parity persistence plan must contain exact save/remove/restore mutations")
  }
  const normalized = source.map((mutation, index) => {
    for (const field of [
      "before",
      "after",
      "afterState",
      "inventory",
      "response",
      "receipt",
      "receipts",
      "successReceipt",
      "mutationReceipt",
      "savedState",
      "saveReceipt",
      "removeReceipt",
    ]) {
      if (mutation && Object.hasOwn(mutation, field)) {
        throw new Error(`managed parity persistence plan must not contain pre-execution ${field} evidence`)
      }
    }
    return normalizePersistenceMutationDefinition(mutation, index, ownedResources)
  })
  const output = redactManagedValue({
    schema: value?.schema ?? MANAGED_PARITY_SCHEMA,
    persistenceMutations: normalized,
  })
  output.ownedResource = stableOwnedIdentity(ownedResources)
  return deepFreeze(output)
}

function normalizePersistenceMutationDefinition(mutation, index, ownedResources = null) {
  const action = PERSISTENCE_MUTATION_ACTIONS[index]
  if (!mutation || mutation.action !== action) {
    throw new Error(`managed parity persistence mutation ${index} must be ${action}`)
  }
  const argv = requireSafeArgv(mutation.argv, `persistence ${action}`)
  const request = mutation.request
  const requestVariant = PERSISTENCE_MUTATION_REQUEST_VARIANTS[action]
  if (!request || typeof request !== "object" || Array.isArray(request)
    || Object.keys(request).length !== 1 || !Object.hasOwn(request, requestVariant)) {
    throw new Error(`managed parity persistence ${action} request must be the exact public ${requestVariant} request`)
  }
  const requestPayload = request[requestVariant]
  const allowedPayloadFields = action === "save"
    ? ["slice_ref", "mode", "scope"]
    : ["slice_ref"]
  if (!requestPayload || typeof requestPayload !== "object" || Array.isArray(requestPayload)
    || Object.keys(requestPayload).some((field) => !allowedPayloadFields.includes(field))) {
    throw new Error(`managed parity persistence ${action} request payload is not the released public shape`)
  }
  if (action === "save"
    && (requestPayload.mode !== "shutdown" || requestPayload.scope !== "this_slice")) {
    throw new Error("managed parity persistence save request must use shutdown for this_slice")
  }
  const requestSliceId = requireText(requestPayload.slice_ref, `${requestVariant}.slice_ref`)
  const ownedSliceId = ownedResources?.sliceId ?? ownedResources?.stableIdentity?.sliceId
  if (hasText(ownedSliceId) && requestSliceId !== ownedSliceId) {
    throw new Error(`managed parity persistence ${action} request targets a different slice identity`)
  }
  const expectedArgv = persistenceRequestArgv(action, request)
  if (!sameArray(argv, expectedArgv)) {
    throw new Error(`managed parity persistence ${action} argv does not describe its exact public request`)
  }
  const requestIdentity = persistenceRequestIdentity(request)
  if (mutation.requestIdentity !== undefined && mutation.requestIdentity !== requestIdentity) {
    throw new Error(`managed parity persistence ${action} request identity does not match its public request`)
  }
  const expectedResponseVariant = PERSISTENCE_MUTATION_RESPONSE_VARIANTS[action]
  if (mutation.responseVariant !== undefined && mutation.responseVariant !== expectedResponseVariant) {
    throw new Error(`managed parity persistence ${action} response variant is not authoritative`)
  }
  const expectedCheckpoint = PERSISTENCE_MUTATION_CHECKPOINTS[action]
  if (!sameJson(mutation.checkpoints, expectedCheckpoint)) {
    throw new Error(`managed parity persistence ${action} checkpoints are not exact`)
  }
  return {
    action,
    argv,
    request: redactManagedValue(request),
    requestIdentity,
    responseVariant: expectedResponseVariant,
    checkpoints: expectedCheckpoint,
  }
}

function normalizePersistenceEvidence(value, ownedResources, plan, capturedLifecycleResults) {
  const source = value?.persistenceMutations ?? value?.mutations
  if (!Array.isArray(source) || source.length !== 3) {
    throw new Error("managed parity persistence must expose exact save/remove/restore mutation evidence")
  }
  const plannedMutations = plan?.persistenceMutations
  if (!Array.isArray(plannedMutations) || plannedMutations.length !== 3) {
    throw new Error("managed parity persistence execution requires an immutable mutation plan")
  }
  if (!Array.isArray(capturedLifecycleResults) || capturedLifecycleResults.length !== plannedMutations.length) {
    throw new Error("managed parity persistence evidence requires captured successful lifecycle responses")
  }
  const normalized = []
  let previousAfter = null
  for (const [index, mutation] of source.entries()) {
    const definition = normalizePersistenceMutationDefinition(mutation, index, ownedResources)
    const plannedDefinition = normalizePersistenceMutationDefinition(plannedMutations[index], index, ownedResources)
    if (!sameJson(definition, plannedDefinition)) {
      throw new Error(`managed parity persistence ${definition.action} result does not match its plan`)
    }
    const { action, argv, request, requestIdentity, responseVariant } = definition
    const captured = capturedLifecycleResults[index]
    if (!captured
      || !sameJson(captured.request, request)
      || !sameJson(captured.response, mutation.response)) {
      throw new Error(`managed parity persistence ${action} evidence is not bound to its captured lifecycle response`)
    }
    const before = mutation.before ?? mutation.inventory
    if (!before || typeof before !== "object" || Array.isArray(before)) {
      throw new Error(`managed parity persistence ${action} requires an authoritative pre-mutation slice state`)
    }
    const after = mutation.after
    if (!after || typeof after !== "object" || Array.isArray(after)) {
      throw new Error(`managed parity persistence ${action} requires an authoritative post-mutation slice state`)
    }
    validateNormalizedPersistenceObservation(action, before, "before", ownedResources)
    validateNormalizedPersistenceObservation(action, after, "after", ownedResources)
    if (previousAfter && !samePersistenceObservationIdentity(previousAfter, before)) {
      throw new Error(`managed parity persistence ${action} evidence is not continuous with the previous mutation`)
    }
    validateNormalizedPersistenceTransition(action, before, after)
    const response = mutation.response
    if (!response || typeof response !== "object" || Array.isArray(response)
      || response.variant !== responseVariant
      || !response.payload || typeof response.payload !== "object" || Array.isArray(response.payload)) {
      throw new Error(`managed parity persistence ${action} requires the actual ${responseVariant} response`)
    }
    const requestVariant = PERSISTENCE_MUTATION_REQUEST_VARIANTS[action]
    const sliceId = requireText(request[requestVariant]?.slice_ref, `${requestVariant}.slice_ref`)
    const responseSlice = requirePersistenceResponseSlice(
      response.payload,
      sliceId,
      `persistence ${action}`,
    )
    if (responseSlice.status !== after.slice.status) {
      throw new Error(`managed parity persistence ${action} response does not match its authoritative after state`)
    }
    if (action === "save") {
      requireSavedState(response.payload.state, sliceId, "persistence save")
    }
    const receipt = mutation.receipt ?? mutation.receipts?.[action]
    if (!receipt || receipt.ok !== true || !hasText(receipt.id ?? receipt.receiptId)) {
      throw new Error(`managed parity persistence ${action} requires a successful receipt`)
    }
    if (receipt.authoritative !== true
      || receipt.action !== action
      || receipt.requestIdentity !== requestIdentity
      || receipt.responseVariant !== responseVariant
      || receipt.responseSliceId !== responseSlice.id) {
      throw new Error(`managed parity persistence ${action} receipt is not bound to its actual request and response`)
    }
    if (action === "save" && !hasText(receipt.archivePath)) {
      throw new Error("managed parity persistence save requires an archive path receipt")
    }
    if (action === "save" && receipt.savedStateId !== response.payload.state?.id) {
      throw new Error("managed parity persistence save receipt has a stale saved-state identity")
    }
    if (action === "remove" && !sameJson(mutation.saveReceipt, normalized[0]?.receipt)) {
      throw new Error("managed parity persistence remove must carry the exact save receipt")
    }
    if (action === "remove" && receipt.parentReceiptId !== receiptId(normalized[0]?.receipt)) {
      throw new Error("managed parity persistence remove receipt must chain from save")
    }
    if (action === "remove" && receipt.savedStateId !== normalized[0]?.receipt?.savedStateId) {
      throw new Error("managed parity persistence remove receipt has a stale saved-state identity")
    }
    if (action === "restore") {
      if (!sameJson(mutation.saveReceipt, normalized[0]?.receipt)
        || !sameJson(mutation.removeReceipt, normalized[1]?.receipt)) {
        throw new Error("managed parity persistence restore must carry exact save/remove receipts")
      }
      if (receipt.parentReceiptId !== receiptId(normalized[1]?.receipt)) {
        throw new Error("managed parity persistence restore receipt must chain from remove")
      }
      if (!hasText(receipt.archivePath)) {
        throw new Error("managed parity persistence restore requires an archive path receipt")
      }
      if (receipt.savedStateId !== normalized[0]?.receipt?.savedStateId) {
        throw new Error("managed parity persistence restore receipt has a stale saved-state identity")
      }
    }
    normalized.push({
      ...redactManagedValue(mutation),
      action,
      argv,
      request,
      checkpoints: definition.checkpoints,
      before: redactManagedValue(before),
      response: redactManagedValue(response),
      after: redactManagedValue(after),
      receipt: redactManagedValue(receipt),
      ...(mutation.saveReceipt ? { saveReceipt: redactManagedValue(mutation.saveReceipt) } : {}),
      ...(mutation.removeReceipt ? { removeReceipt: redactManagedValue(mutation.removeReceipt) } : {}),
    })
    previousAfter = after
  }
  if (!samePersistenceObservationIdentity(normalized[0]?.before, normalized.at(-1)?.after)) {
    throw new Error("managed parity persistence evidence changed its final authoritative identity")
  }
  const output = redactManagedValue({
    ...(value && typeof value === "object" && !Array.isArray(value) ? value : {}),
    schema: value?.schema ?? MANAGED_PARITY_SCHEMA,
    persistenceMutations: normalized,
  })
  // Keep the ownership identity as a separate, stable redacted field. It lets
  // cleanup inspection correlate receipts without trusting caller-supplied IDs.
  output.ownedResource = stableOwnedIdentity(ownedResources)
  return output
}

function validateNormalizedPersistenceObservation(action, observation, phase, ownedResources) {
  const slice = observation?.slice
  if (!slice || typeof slice !== "object" || Array.isArray(slice)) {
    throw new Error(`managed parity persistence ${action} ${phase} evidence requires an authoritative slice state`)
  }
  const ownedSliceId = ownedResources?.sliceId ?? ownedResources?.stableIdentity?.sliceId
  if (!hasText(slice.id) || (hasText(ownedSliceId) && slice.id !== ownedSliceId)) {
    throw new Error(`managed parity persistence ${action} ${phase} evidence has a foreign slice identity`)
  }
  if (!hasText(slice.status) || !hasText(slice.environment_session_id)) {
    throw new Error(`managed parity persistence ${action} ${phase} evidence lacks authoritative slice identity or status`)
  }
  const inventory = observation.inventory
  if (!inventory) {
    if (slice.status !== "stopped") {
      throw new Error(`managed parity persistence ${action} ${phase} evidence lacks authoritative resource identity`)
    }
    return
  }
  if (typeof inventory !== "object" || Array.isArray(inventory)
    || inventory.slice_id !== slice.id
    || !hasText(inventory.environment_id)) {
    throw new Error(`managed parity persistence ${action} ${phase} evidence lacks authoritative resource identity`)
  }
  requireUniqueIdentityArray(inventory.browser_ids, `persistence ${action} ${phase} browser_ids`)
  requireUniqueIdentityArray(inventory.profile_ids, `persistence ${action} ${phase} profile_ids`)
}

function validateNormalizedPersistenceTransition(action, before, after) {
  const expectedStatuses = {
    save: { before: "running", after: "stopped" },
    remove: { before: "stopped", after: "stopped" },
    restore: { before: "stopped", after: "running" },
  }[action]
  if (before.slice.status !== expectedStatuses.before || after.slice.status !== expectedStatuses.after) {
    throw new Error(`managed parity persistence ${action} evidence has an invalid authoritative state transition`)
  }
  if (!samePersistenceObservationIdentity(before, after)) {
    throw new Error(`managed parity persistence ${action} evidence changed its authoritative identity`)
  }
}

function receiptId(receipt) {
  return receipt?.id ?? receipt?.receiptId ?? null
}

async function inspectCleanupResidue({
  displayClient,
  requestApi,
  ownedResources,
  residueAdapter,
  request,
  signal,
}) {
  const evidence = ownedResources.cleanupEvidence
  if (!evidence?.cleaned) {
    throw new Error("managed parity cleanup.inspect requires a completed cleanup.perform")
  }
  if (evidence.roomDeleted !== true) {
    throw new Error("managed parity cleanup.inspect requires an observed deleted drill Room")
  }
  if (!hasText(evidence.sliceId)) {
    throw new Error("managed parity cleanup.inspect requires a stable owned slice identity")
  }
  const publicInspection = await inspectOwnedResidue({
    displayClient,
    requestApi,
    ownedResources,
    evidence,
    signal,
  })
  const managedInspection = residueAdapter
    ? await invokeManagedAdapter(residueAdapter, ["inspect", "inspectCleanupResidue", "cleanupInspect"], {
      client: displayClient,
      requestApi,
      ownedResource: stableOwnedIdentity(ownedResources),
      cleanupEvidence: redactManagedValue(evidence),
      request: redactManagedValue(request ?? {}),
      signal,
    }, "cleanup inspection")
    : null
  if (!managedInspection) {
    throw new Error("managed parity cleanup.inspect requires measured post-delete cleanup evidence")
  }
  const output = normalizeCleanupInspection({
    ...(managedInspection && typeof managedInspection === "object" ? managedInspection : {}),
    ...publicInspection,
    ...(managedInspection && typeof managedInspection === "object"
      ? {
        zeroResidue: managedInspection.zeroResidue === true && publicInspection.zeroResidue === true,
        unsupportedChecks: [
          ...(managedInspection.unsupportedChecks ?? []),
        ],
      }
      : {}),
    owned: stableOwnedIdentity(ownedResources),
  }, { requireManagedMetrics: true })
  if (output.zeroResidue !== true) {
    throw new Error("managed parity cleanup.inspect found owned resource residue")
  }
  return output
}

async function inspectOwnedResidue({ displayClient, requestApi, ownedResources, evidence, signal }) {
  const sliceId = evidence.sliceId
  const slicesResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "listSlicesRequest")(),
    signal,
    "cleanup.inspect slices",
  )
  const slices = requireArray(
    responseVariant(slicesResponse, "SlicesListed", "cleanup.inspect slices").slices,
    "cleanup.inspect SlicesListed.slices",
  )
  const ownedSlices = slices.filter((slice) => slice?.id === sliceId)
  if (ownedSlices.length > 0) {
    throw new Error("managed parity cleanup.inspect found the owned slice still present")
  }

  const sessionsResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "listSessionsRequest")(),
    signal,
    "cleanup.inspect sessions",
  )
  const sessions = requireArray(
    responseVariant(sessionsResponse, "SessionsListed", "cleanup.inspect sessions").sessions,
    "cleanup.inspect SessionsListed.sessions",
  )
  const roomId = evidence.identity?.roomId ?? evidence.identity?.sessionId
  const roomSessions = sessions.filter((session) => session?.id === roomId)
  if (roomSessions.length > 0) {
    throw new Error("managed parity cleanup.inspect found the owned Room still present")
  }
  const attachmentIds = new Set(evidence.attachmentIds ?? [])
  if (ownedResourcesTrackedAfterCleanup(ownedResources)) {
    throw new Error("managed parity cleanup.inspect found tracked owned resources")
  }
  if (attachmentIds.size !== new Set(evidence.detachedAttachmentIds ?? []).size) {
    throw new Error("managed parity cleanup.inspect lacks a detach receipt for every attachment")
  }
  const residualAttachmentIds = new Set()
  for (const session of sessions) {
    for (const id of session?.attachment_ids ?? session?.attachmentIds ?? []) {
      if (attachmentIds.has(id)) residualAttachmentIds.add(id)
    }
  }
  let memberInspection = "not-requested"
  if (roomSessions.length > 0 && hasText(roomId) && typeof requestApi.listSessionMembersRequest === "function") {
    const membersResponse = await sendWithAbortSignal(
      displayClient,
      requestApi.listSessionMembersRequest(roomId),
      signal,
      "cleanup.inspect session members",
    )
    const membersPayload = responseVariant(
      membersResponse,
      "SessionMembersListed",
      "cleanup.inspect session members",
    )
    const members = requireArray(membersPayload.members, "cleanup.inspect SessionMembersListed.members")
    for (const member of members.filter((member) => {
      const id = member?.attachment_id ?? member?.attachmentId ?? member?.id
      return attachmentIds.has(id)
    })) {
      residualAttachmentIds.add(member.attachment_id ?? member.attachmentId ?? member.id)
    }
    memberInspection = "public-session-members"
  } else {
    memberInspection = "public-session-list-only"
  }
  const attachmentResidueCount = residualAttachmentIds.size
  if (attachmentResidueCount > 0) {
    throw new Error("managed parity cleanup.inspect found an owned session attachment still present")
  }
  return {
    inspected: true,
    zeroResidue: true,
    owned: {
      sliceId: evidence.sliceId,
      attachmentIds: [...attachmentIds],
      detachedAttachmentIds: [...new Set(evidence.detachedAttachmentIds ?? [])],
      deleted: evidence.deleted === true,
    },
    publicInventory: {
      ownedSliceCount: ownedSlices.length,
      roomPresent: sessions.some((session) => session?.id === roomId),
      receiptKinds: ["SlicesListed", "SessionsListed", "SliceDeleted", "SessionDetached"],
    },
    ownedSliceCount: ownedSlices.length,
    ownedAttachmentResidueCount: attachmentResidueCount,
    sessionCount: sessions.filter((session) => session?.id === roomId).length,
    memberInspection,
    unsupportedChecks: [],
  }
}

function ownedResourcesTrackedAfterCleanup(ownedResources) {
  return Boolean(ownedResources.sliceId)
    || ownedResources.attachmentIds.size !== 0
    || ownedResources.attachmentsByClient.size !== 0
    || ownedResources.agentIds.size !== 0
}

function normalizeCleanupInspection(value, { requireManagedMetrics = false } = {}) {
  const output = redactManagedValue({
    schema: value?.schema ?? MANAGED_PARITY_SCHEMA,
    inspected: value?.inspected === true,
    zeroResidue: value?.zeroResidue === true,
    owned: value?.owned ?? null,
    publicInventory: value?.publicInventory ?? null,
    ownedSliceCount: requireNonNegativeInteger(value?.ownedSliceCount, "cleanup ownedSliceCount"),
    ownedAttachmentResidueCount: requireNonNegativeInteger(
      value?.ownedAttachmentResidueCount,
      "cleanup ownedAttachmentResidueCount",
    ),
    sessionCount: requireNonNegativeInteger(value?.sessionCount, "cleanup sessionCount"),
    memberInspection: value?.memberInspection ?? "unknown",
    unsupportedChecks: Array.isArray(value?.unsupportedChecks) ? value.unsupportedChecks : [],
    ...(value?.resources === undefined
      ? {}
      : {
        resources: {
          rssDeltaBytes: requireFiniteNumber(
            value.resources?.rssDeltaBytes,
            "cleanup resources.rssDeltaBytes",
          ),
          diskDeltaBytes: requireFiniteNumber(
            value.resources?.diskDeltaBytes,
            "cleanup resources.diskDeltaBytes",
          ),
        },
      }),
  })
  const managedCountFields = [
    "managedMachines", "rooms", "environments", "processes", "listeners", "containers", "profiles",
    "activeTargets", "temporaryFiles", "retainedEvidenceLeakCount",
  ]
  for (const field of managedCountFields) {
    if (value?.[field] !== undefined) {
      output[field] = requireNonNegativeInteger(value[field], `cleanup ${field}`)
    } else if (requireManagedMetrics) {
      throw new Error(`managed parity cleanup ${field} must be a finite non-negative integer`)
    }
  }
  if (requireManagedMetrics && value?.resources === undefined) {
    throw new Error("managed parity cleanup resources must contain finite non-negative deltas")
  }
  return output
}

function stableOwnedIdentity(ownedResources) {
  const identity = ownedResources.stableIdentity ?? ownedResources.identity
  if (!identity && !ownedResources.cleanupEvidence) return null
  return redactManagedValue({
    ...(identity ?? {}),
    sliceId: identity?.sliceId ?? ownedResources.cleanupEvidence?.sliceId ?? ownedResources.sliceId ?? null,
    attachmentIds: [...(ownedResources.cleanupEvidence?.attachmentIds
      ?? ownedResources.attachmentIds
      ?? [])],
    detachedAttachmentIds: [...(ownedResources.cleanupEvidence?.detachedAttachmentIds
      ?? ownedResources.detachedAttachmentIds
      ?? [])],
    agentIds: [...(ownedResources.cleanupEvidence?.agentIds
      ?? ownedResources.agentIds
      ?? [])],
    deleted: ownedResources.cleanupEvidence?.deleted === true,
  })
}

function firstCallable(value, names) {
  return names.map((name) => value?.[name]).find((candidate) => typeof candidate === "function") ?? null
}

function firstAdapterMethod(adapter, names) {
  if (typeof adapter === "function") return adapter
  return firstCallable(adapter, names)
}

async function invokeManagedAdapter(adapter, names, input, step) {
  const method = firstAdapterMethod(adapter, names)
  if (!method) throw new Error(`managed parity ${step} seam is not callable`)
  return method.call(adapter, input)
}

function redactManagedValue(value, key = "") {
  if (isSensitiveKey(key)) return "[REDACTED]"
  if (Array.isArray(value)) return value.map((entry) => redactManagedValue(entry))
  if (!value || typeof value !== "object") return value
  const result = {}
  for (const [entryKey, entryValue] of Object.entries(value)) {
    result[entryKey] = redactManagedValue(entryValue, entryKey)
  }
  return result
}

function deepFreeze(value) {
  if (!value || typeof value !== "object" || Object.isFrozen(value)) return value
  for (const child of Object.values(value)) deepFreeze(child)
  return Object.freeze(value)
}

function isSensitiveKey(key) {
  return /(?:token|secret|password|credential|authorization|provider.?auth|api.?key|private.?key|access.?key)/i.test(key)
}

function requireSafeArgv(value, label) {
  if (!Array.isArray(value) || value.length < 2 || value.some((entry) => typeof entry !== "string")) {
    throw new Error(`managed parity ${label} requires an exact argv array`)
  }
  for (const entry of value) {
    if (/(?:token|secret|password|authorization|provider.?auth|api.?key)=/i.test(entry)) {
      throw new Error(`managed parity ${label} argv must not contain credentials`)
    }
  }
  return [...value]
}

function sameArray(left, right) {
  return Array.isArray(left) && Array.isArray(right)
    && left.length === right.length && left.every((entry, index) => entry === right[index])
}

function sameJson(left, right) {
  return JSON.stringify(left) === JSON.stringify(right)
}

function normalizeTimestamp(value) {
  if (value instanceof Date && !Number.isNaN(value.valueOf())) return value.toISOString()
  if (typeof value === "function") {
    const result = value()
    if (result instanceof Date && !Number.isNaN(result.valueOf())) return result.toISOString()
    if (typeof result === "string" && !Number.isNaN(Date.parse(result))) return new Date(result).toISOString()
  }
  if (typeof value === "string" && !Number.isNaN(Date.parse(value))) return new Date(value).toISOString()
  return new Date().toISOString()
}

function requireNonNegativeFinite(value, label) {
  if (!Number.isFinite(value) || value < 0) {
    throw new Error(`managed parity ${label} must be a finite non-negative number`)
  }
  return value
}

function requireFiniteNumber(value, label) {
  if (!Number.isFinite(value)) {
    throw new Error(`managed parity ${label} must be a finite number`)
  }
  return value
}

function requireNonNegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`managed parity ${label} must be a finite non-negative integer`)
  }
  return value
}

function finiteTimeout(value, fallback, maximum = 120_000) {
  return Number.isFinite(value) && value > 0 ? Math.min(value, maximum) : fallback
}

async function withDeadline(operation, { signal, timeoutMs, step }) {
  if (signal?.aborted) throw abortError(step)
  const controller = new AbortController()
  let timer
  let abort
  const operationPromise = Promise.resolve().then(() => operation(controller.signal))
  const timeoutPromise = new Promise((_, reject) => {
    timer = setTimeout(() => {
      const timeoutError = new Error(`managed parity ${step} timed out after ${timeoutMs}ms`)
      controller.abort(timeoutError)
      reject(timeoutError)
    }, timeoutMs)
  })
  const abortedPromise = signal
    ? new Promise((_, reject) => {
      abort = () => {
        const error = abortError(step)
        controller.abort(signal.reason ?? error)
        reject(error)
      }
      signal.addEventListener("abort", abort, { once: true })
      if (signal.aborted) abort()
    })
    : null
  try {
    return await Promise.race([operationPromise, timeoutPromise, ...(abortedPromise ? [abortedPromise] : [])])
  } finally {
    clearTimeout(timer)
    if (abort) signal.removeEventListener("abort", abort)
  }
}

async function runSelkiesCreate({
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  evidenceRoot,
  request,
  signal,
  displayBackend = "selkies",
  sliceLifecycleTimeoutMs,
}) {
  if (displayBackend !== "selkies") {
    return runDisplayBackendCreate({
      displayClient,
      identityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      ownedResources,
      evidenceRoot,
      request,
      signal,
      displayBackend,
      sliceLifecycleTimeoutMs,
    })
  }
  if (ownedResources.sliceId) {
    throw new Error("managed parity selkies.create already owns a slice; duplicate creation is not allowed")
  }
  const binding = requireBinding(request, "selkies.create")
  if (request.kernelOwnedDefault !== true) {
    throw new Error("managed parity selkies.create requires the kernel-owned default")
  }
  const workerKernelRef = requireText(targetKernelRef, "managed parity target worker kernel reference")
  const roomId = requireText(binding.roomId, "binding.roomId")
  const runId = requireText(request.runId, "runId")
  ownedResources.expectedRoomId = roomId
  ownedResources.evidenceRoot = hasText(evidenceRoot) ? evidenceRoot.trim() : ownedResources.evidenceRoot
  ownedResources.cleanupMeasurements = await createCleanupMeasurementBaseline({
    evidenceRoot: ownedResources.evidenceRoot,
    signal,
  })
  if (request.displayBackend !== null && request.displayBackend !== undefined) {
    throw new Error("managed parity selkies.create requires the kernel-owned display backend default")
  }
  const createResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.createSliceRequest({
      name: `${runId}-selkies`,
      backend: "ssh_docker",
      displayMode: "headed",
      workerKernelRef,
      base: "clean",
    }),
    signal,
    "selkies.create",
    sliceLifecycleTimeoutMs,
  )
  const created = responseVariant(createResponse, "SliceCreated", "selkies.create")?.slice
  const createdSliceId = requireText(created?.id, "SliceCreated.slice.id")
  // Record the returned ownership identity before any deeper response
  // validation so cleanup can still delete a resource after a partial failure.
  ownedResources.sliceId = createdSliceId
  ownedResources.detachedAttachmentIds.clear()
  ownedResources.cleanupEvidence = null
  validateCreatedSlice(created, {
    roomId,
    workerKernelRef,
    targetMachineRef,
    step: "selkies.create",
  })

  const bindResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.bindRoomEnvironmentSliceRequest(roomId, ownedResources.sliceId),
    signal,
    "selkies.create Room binding",
  )
  const bindingResponse = responseVariant(
    bindResponse,
    "RoomEnvironmentSlice",
    "selkies.create Room binding",
  ).binding
  validateRoomSliceBinding(bindingResponse, {
    roomId,
    sliceId: ownedResources.sliceId,
    workerKernelRef,
  })

  const startResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.startSliceRequest(ownedResources.sliceId),
    signal,
    "selkies.create slice start",
    sliceLifecycleTimeoutMs,
  )
  const started = responseVariant(
    startResponse,
    "SliceStarted",
    "selkies.create slice start",
  ).slice
  validateStartedSlice(started, {
    sliceId: ownedResources.sliceId,
    roomId,
    workerKernelRef,
    binding,
    targetMachineRef,
    step: "selkies.create slice start",
  })

  // Create responses for a newly provisioned slice may intentionally omit the
  // display endpoint until the worker is running. Observe the post-start
  // record through the released public GetSlice request instead of inferring
  // the kernel-selected backend from the stopped CreateSlice response.
  const observedResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "getSliceRequest")(ownedResources.sliceId),
    signal,
    "selkies.create slice observation",
  )
  const observed = responseVariant(
    observedResponse,
    "Slice",
    "selkies.create slice observation",
  ).slice
  validateStartedSlice(observed, {
    sliceId: ownedResources.sliceId,
    roomId,
    workerKernelRef,
    binding,
    targetMachineRef,
    step: "selkies.create slice observation",
  })
  const observedBackend = observedDisplayBackend(observed, "selkies.create slice observation")

  const identity = await readAuthoritativeBinding({
    displayClient,
    identityClient,
    requestApi,
    roomId,
    signal,
    step: "selkies.create",
  })
  assertBinding(identity, binding, "selkies.create")
  const resourceCounts = await observeAuthoritativeResourceCounts({
    displayClient,
    requestApi,
    roomId,
    environmentId: binding.environmentId,
    sliceId: ownedResources.sliceId,
    signal,
    step: "selkies.create",
  })
  ownedResources.identity = identity
  ownedResources.stableIdentity = {
    ...identity,
    sliceId: ownedResources.sliceId,
  }
  return {
    ...identity,
    displayBackend: observedBackend,
    sliceId: ownedResources.sliceId,
    ...resourceCounts,
  }
}

async function runDisplayBackendCreate({
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  ownedResources,
  evidenceRoot,
  request,
  signal,
  displayBackend,
  sliceLifecycleTimeoutMs,
}) {
  if (displayBackend === "selkies") {
    return runSelkiesCreate({
      displayClient,
      identityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      ownedResources,
      evidenceRoot,
      request,
      signal,
      displayBackend,
      sliceLifecycleTimeoutMs,
    })
  }
  if (displayBackend !== "novnc") {
    throw new Error(`managed parity create does not support display backend ${displayBackend}`)
  }
  if (ownedResources.sliceId) {
    throw new Error("managed parity novnc.create already owns a slice; duplicate creation is not allowed")
  }
  const binding = requireBinding(request, "novnc.create")
  if (request.kernelOwnedDefault === true || request.displayBackend !== "novnc") {
    throw new Error("managed parity novnc.create requires the explicit novnc display backend")
  }
  const workerKernelRef = requireText(targetKernelRef, "managed parity target worker kernel reference")
  const roomId = requireText(binding.roomId, "binding.roomId")
  const runId = requireText(request.runId, "runId")
  ownedResources.expectedRoomId = roomId
  ownedResources.evidenceRoot = hasText(evidenceRoot) ? evidenceRoot.trim() : ownedResources.evidenceRoot
  ownedResources.cleanupMeasurements = await createCleanupMeasurementBaseline({
    evidenceRoot: ownedResources.evidenceRoot,
    signal,
  })
  const createResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.createSliceRequest({
      name: `${runId}-novnc`,
      backend: "ssh_docker",
      displayMode: "headed",
      displayBackend: "novnc",
      workerKernelRef,
      base: "clean",
    }),
    signal,
    "novnc.create",
    sliceLifecycleTimeoutMs,
  )
  const created = responseVariant(createResponse, "SliceCreated", "novnc.create")?.slice
  ownedResources.sliceId = requireText(created?.id, "SliceCreated.slice.id")
  ownedResources.detachedAttachmentIds.clear()
  ownedResources.cleanupEvidence = null
  validateCreatedSlice(created, {
    roomId,
    workerKernelRef,
    targetMachineRef,
    step: "novnc.create",
  })
  const bindResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.bindRoomEnvironmentSliceRequest(roomId, ownedResources.sliceId),
    signal,
    "novnc.create Room binding",
  )
  validateRoomSliceBinding(
    responseVariant(bindResponse, "RoomEnvironmentSlice", "novnc.create Room binding").binding,
    { roomId, sliceId: ownedResources.sliceId, workerKernelRef },
  )
  const startResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.startSliceRequest(ownedResources.sliceId),
    signal,
    "novnc.create slice start",
    sliceLifecycleTimeoutMs,
  )
  const started = responseVariant(startResponse, "SliceStarted", "novnc.create slice start").slice
  validateStartedSlice(started, {
    sliceId: ownedResources.sliceId,
    roomId,
    workerKernelRef,
    binding,
    targetMachineRef,
    step: "novnc.create slice start",
  })
  const observedResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "getSliceRequest")(ownedResources.sliceId),
    signal,
    "novnc.create slice observation",
  )
  const observed = responseVariant(observedResponse, "Slice", "novnc.create slice observation").slice
  validateStartedSlice(observed, {
    sliceId: ownedResources.sliceId,
    roomId,
    workerKernelRef,
    binding,
    targetMachineRef,
    step: "novnc.create slice observation",
  })
  const observedBackend = observedDisplayBackend(observed, "novnc.create slice observation", "novnc")
  const identity = await readAuthoritativeBinding({
    displayClient,
    identityClient,
    requestApi,
    roomId,
    signal,
    step: "novnc.create",
  })
  assertBinding(identity, binding, "novnc.create")
  const resourceCounts = await observeAuthoritativeResourceCounts({
    displayClient,
    requestApi,
    roomId,
    environmentId: binding.environmentId,
    sliceId: ownedResources.sliceId,
    signal,
    step: "novnc.create",
  })
  ownedResources.identity = identity
  ownedResources.stableIdentity = { ...identity, sliceId: ownedResources.sliceId }
  return {
    ...identity,
    displayBackend: observedBackend,
    sliceId: ownedResources.sliceId,
    ...resourceCounts,
  }
}

async function runSelkiesDestroy({
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  telemetryAdapter,
  ownedResources,
  request,
  signal,
  displayBackend = "selkies",
  sliceLifecycleTimeoutMs,
}) {
  const step = `${displayBackend}.destroy`
  const sliceId = resolveOwnedSliceId(request, ownedResources, step, { requireOwned: true })
  const identity = ownedResources.identity
  if (!identity) {
    throw new Error(`managed parity ${step} cannot verify the created target identity`)
  }
  const attachmentIds = [...ownedResources.attachmentIds]
  await captureCleanupMeasurements({
    displayClient,
    identityClient,
    requestApi,
    targetKernelRef,
    targetMachineRef,
    telemetryAdapter,
    ownedResources,
    sliceId,
    signal,
    step,
  })
  await detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step })
  await retireOwnedAgents({
    displayClient,
    requestApi,
    ownedResources,
    sliceId,
    roomId: identity.roomId,
    signal,
    step,
  })
  await deleteOwnedSlice({ displayClient, requestApi, sliceId, signal, step, sliceLifecycleTimeoutMs })
  ownedResources.cleanupMeasurements.postDelete = await observePostDeleteRoom({
    displayClient,
    requestApi,
    roomId: identity.roomId,
    sliceId,
    signal,
    step,
  })
  rememberCleanupEvidence(ownedResources, {
    sliceId,
    attachmentIds,
    identity,
    deleted: true,
    cleaned: false,
    roomDeleted: false,
    reason: "destroy",
  })
  clearOwnedResources(ownedResources)
  return {
    ...identity,
    displayBackend,
    destroyed: true,
    sliceId,
    attachmentIds,
  }
}

async function runCleanup({
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  telemetryAdapter,
  ownedResources,
  signal,
  sliceLifecycleTimeoutMs,
}) {
  const sliceId = ownedResources.sliceId
  const attachmentIds = [...ownedResources.attachmentIds]
  if (sliceId) {
    const roomId = ownedResources.identity?.roomId ?? ownedResources.expectedRoomId
    await captureCleanupMeasurements({
      displayClient,
      identityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      telemetryAdapter,
      ownedResources,
      sliceId,
      signal,
      step: "cleanup.perform",
    })
    await detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step: "cleanup.perform" })
    await retireOwnedAgents({
      displayClient,
      requestApi,
      ownedResources,
      sliceId,
      roomId,
      signal,
      step: "cleanup.perform",
    })
    await deleteOwnedSlice({
      displayClient,
      requestApi,
      sliceId,
      signal,
      step: "cleanup.perform",
      sliceLifecycleTimeoutMs,
    })
    ownedResources.cleanupMeasurements.postDelete = await observePostDeleteRoom({
      displayClient,
      requestApi,
      roomId,
      sliceId,
      signal,
      step: "cleanup.perform",
    })
  }
  const previousEvidence = ownedResources.cleanupEvidence
  const roomId = previousEvidence?.identity?.roomId
    ?? ownedResources.identity?.roomId
    ?? ownedResources.expectedRoomId
  const roomDeleted = await deleteOwnedRoom({
    displayClient,
    requestApi,
    roomId,
    signal,
    step: "cleanup.perform",
  })
  rememberCleanupEvidence(ownedResources, {
    sliceId: sliceId ?? previousEvidence?.sliceId,
    attachmentIds: attachmentIds.length > 0 ? attachmentIds : previousEvidence?.attachmentIds,
    identity: ownedResources.identity ?? previousEvidence?.identity ?? ownedResources.stableIdentity,
    deleted: true,
    cleaned: true,
    roomDeleted,
    reason: "cleanup",
  })
  clearOwnedResources(ownedResources)
  return { cleaned: true, sliceId, attachmentIds }
}

async function createCleanupMeasurementBaseline({ evidenceRoot, signal }) {
  return {
    evidenceBefore: await measureEvidenceInventory(evidenceRoot, signal),
    beforeInventory: null,
    beforeTelemetry: null,
    postDelete: null,
  }
}

async function captureCleanupMeasurements({
  displayClient,
  identityClient,
  requestApi,
  targetKernelRef,
  targetMachineRef,
  telemetryAdapter,
  ownedResources,
  sliceId,
  signal,
  step,
}) {
  if (!ownedResources.cleanupMeasurements) {
    ownedResources.cleanupMeasurements = await createCleanupMeasurementBaseline({
      evidenceRoot: ownedResources.evidenceRoot,
      signal,
    })
  }
  const roomId = ownedResources.identity?.roomId ?? ownedResources.expectedRoomId
  if (!ownedResources.cleanupMeasurements.beforeInventory && hasText(roomId)) {
    ownedResources.cleanupMeasurements.beforeInventory = await readCleanupInventoryBefore({
      client: displayClient,
      requestApi,
      roomId,
      sliceId,
      signal,
      step: `${step} measured inventory before cleanup`,
    })
  }
  if (!ownedResources.cleanupMeasurements.beforeTelemetry
    && hasManagedResourceTelemetryPath(identityClient, requestApi, telemetryAdapter)) {
    ownedResources.cleanupMeasurements.beforeTelemetry = await collectManagedTargetResourceSnapshot({
      client: identityClient,
      requestApi,
      targetKernelRef,
      targetMachineRef,
      ownedResources,
      telemetryAdapter,
      phase: "cleanup-before",
      sampleId: `cleanup:${sliceId}:before`,
      evidenceRoot: ownedResources.evidenceRoot,
      signal,
    })
  }
}

async function readCleanupInventoryBefore({ client, requestApi, roomId, sliceId, signal, step }) {
  const slicesResponse = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listSlicesRequest")(),
    signal,
    `${step} slices`,
  )
  const slices = requireArray(responseVariant(slicesResponse, "SlicesListed", `${step} slices`).slices, `${step} slices`)
  const sessions = await readSessionRecords({ client, requestApi, signal, step: `${step} Rooms` })
  const slice = slices.find((item) => item?.id === sliceId)
  let resourceInventory = null
  if (slice && (slice.environment_session_id === roomId || slice.session_id === roomId)
    && typeof requestApi.getRoomEnvironmentResourceInventoryRequest === "function") {
    const response = await sendWithAbortSignal(
      client,
      requestApi.getRoomEnvironmentResourceInventoryRequest(roomId, sliceId),
      signal,
      `${step} browser/profile inventory`,
    )
    const inventory = responseVariant(
      response,
      "RoomEnvironmentResourceInventory",
      `${step} browser/profile inventory`,
    ).inventory
    resourceInventory = {
      ...inventory,
      browser_ids: requireIdentityValues(inventory?.browser_ids, `${step} browser_ids`),
      profile_ids: requireIdentityValues(inventory?.profile_ids, `${step} profile_ids`),
    }
  }
  let environment = null
  if (typeof requestApi.getRoomEnvironmentStateRequest === "function") {
    const response = await sendWithAbortSignal(
      client,
      requestApi.getRoomEnvironmentStateRequest(roomId),
      signal,
      `${step} Room state`,
    )
    environment = responseVariant(response, "RoomEnvironmentState", `${step} Room state`).environment
    if (environment?.session_id !== roomId) throw new Error(`${step} returned a foreign Room`)
  }
  return {
    slices: redactManagedValue(slices),
    sessions: redactManagedValue(sessions),
    resourceInventory: redactManagedValue(resourceInventory),
    environment: summarizeEnvironment(environment),
  }
}

async function observePostDeleteRoom({ displayClient, requestApi, roomId, sliceId, signal, step }) {
  const response = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "getRoomEnvironmentSliceRequest")(roomId),
    signal,
    `${step} unbound Room observation`,
  )
  const payload = responseVariant(response, "RoomEnvironmentSlice", `${step} unbound Room observation`)
  if (!Object.hasOwn(payload, "binding")) {
    throw new Error(`${step} unbound Room observation omitted the binding state`)
  }
  if (payload.binding !== null && payload.binding !== undefined) {
    throw new Error(`${step} slice deletion left the Room bound to a slice`)
  }
  const stateResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "getRoomEnvironmentStateRequest")(roomId),
    signal,
    `${step} post-delete Room state`,
  )
  const environment = responseVariant(stateResponse, "RoomEnvironmentState", `${step} post-delete Room state`).environment
  if (environment?.session_id !== roomId) throw new Error(`${step} post-delete Room state returned a foreign Room`)
  const summary = summarizeEnvironment(environment)
  return {
    bindingPresent: false,
    sliceId,
    browserIds: [],
    profileIds: [],
    ...summary,
  }
}

function summarizeEnvironment(environment) {
  if (!environment) {
    return {
      lifecycle: null,
      activeEnvironmentCount: 0,
      activeProcessCount: 0,
      inputResidueCount: 0,
    }
  }
  const health = requireArray(environment.health, "cleanup environment health")
  const activeHealth = health.filter((item) => ["starting", "ready", "degraded"].includes(item?.state))
  const activeControllerResources = activeHealth.filter((item) => [
    "browser_controller", "browser", "desktop", "streamer",
  ].includes(item?.component))
  const activeActions = requireArray(environment.actions, "cleanup environment actions")
    .filter((action) => ["queued", "running"].includes(action?.state))
  const inputResidueCount = requireArray(environment.input_ownership ?? [], "cleanup input ownership").length
    + requireArray(environment.pending_input_takeovers ?? [], "cleanup pending input takeovers").length
  return {
    lifecycle: environment.lifecycle,
    activeEnvironmentCount: environment.lifecycle === "stopped" ? 0 : 1,
    activeProcessCount: activeControllerResources.length + activeActions.length,
    inputResidueCount,
  }
}

async function measureEvidenceInventory(root, signal) {
  if (!hasText(root)) return null
  const paths = new Set()
  let bytes = 0
  const walk = async (directory) => {
    if (signal?.aborted) throw abortError("evidence inventory")
    let entries
    try {
      entries = await readdir(directory, { withFileTypes: true })
    } catch (error) {
      if (error?.code === "ENOENT") return
      throw error
    }
    for (const entry of entries) {
      const path = `${directory}/${entry.name}`
      if (entry.isDirectory()) {
        await walk(path)
      } else if (entry.isFile()) {
        const details = await stat(path)
        paths.add(path)
        bytes += details.size
      }
    }
  }
  await walk(root.trim())
  return { paths: [...paths].sort(), bytes, fileCount: paths.size }
}

async function measureEvidenceDelta(before, root) {
  if (!before || !hasText(root)) {
    throw new Error("managed parity cleanup.inspect requires an evidence inventory root")
  }
  const after = await measureEvidenceInventory(root)
  if (!after) {
    throw new Error("managed parity cleanup.inspect could not observe the evidence inventory after cleanup")
  }
  const oldPaths = new Set(before.paths)
  const newPaths = after.paths.filter((path) => !oldPaths.has(path))
  return {
    newPathCount: newPaths.length,
    newBytes: after.bytes - before.bytes,
  }
}

async function deleteOwnedSlice({
  displayClient,
  requestApi,
  sliceId,
  signal,
  step,
  sliceLifecycleTimeoutMs,
}) {
  const before = await readAuthoritativeSliceRecord({
    client: displayClient,
    requestApi,
    sliceId,
    signal,
    step: `${step} slice membership before delete`,
  })
  if (!before) return
  let deleteError
  try {
    const deleteResponse = await sendWithAbortSignal(
      displayClient,
      requireRequestConstructor(requestApi, "deleteSliceRequest")(sliceId),
      signal,
      step,
      sliceLifecycleTimeoutMs,
    )
    const deleted = responseVariant(deleteResponse, "SliceDeleted", step).slice
    validateDeletedSlice(deleted, sliceId, step)
  } catch (error) {
    deleteError = error
  }
  const after = await readAuthoritativeSliceRecord({
    client: displayClient,
    requestApi,
    sliceId,
    signal,
    step: `${step} slice deletion observation`,
  })
  if (after) throw deleteError ?? new Error(`managed parity ${step} left the owned slice present`)
}

async function retireOwnedAgents({ displayClient, requestApi, ownedResources, sliceId, roomId, signal, step }) {
  if (typeof requestApi?.listSlicesRequest !== "function") {
    throw new Error(`managed parity ${step} requires authoritative slice membership before deletion`)
  }
  const membership = await readAuthoritativeSliceRecord({
    client: displayClient,
    requestApi,
    sliceId,
    signal,
    step: `${step} agent membership`,
  })
  if (!membership) return
  if (membership.agentIds.length === 0) {
    ownedResources.agentIds.clear()
    return
  }
  const destroyAgent = requireRequestConstructor(requestApi, "destroyAgentRequest")
  const tracked = new Set(ownedResources.agentIds)
  const untracked = membership.agentIds.filter((agentId) => !tracked.has(agentId))
  if (untracked.length > 0) {
    throw new Error(`${step} found untracked agents on the owned slice: ${untracked.join(",")}`)
  }
  for (const agentId of membership.agentIds) {
    const response = await sendWithAbortSignal(
      displayClient,
      destroyAgent(roomId, agentId),
      signal,
      `${step} agent retirement`,
    )
    const destroyed = responseVariant(response, "AgentDestroyed", `${step} agent retirement`).agent
    if (destroyed?.id !== agentId) {
      throw new Error(`${step} agent retirement returned a different agent identity`)
    }
  }
  await waitForKernelProbe(
    async () => {
      const latest = await readAuthoritativeSliceRecord({
        client: displayClient,
        requestApi,
        sliceId,
        signal,
        step: `${step} agent membership after retirement`,
      })
      if (!latest || latest.agentIds.length === 0) {
        ownedResources.agentIds.clear()
        return true
      }
      return false
    },
    5_000,
    `${step} agent retirement`,
    signal,
  )
}

async function readAuthoritativeSliceRecord({ client, requestApi, sliceId, signal, step }) {
  const response = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listSlicesRequest")(),
    signal,
    step,
  )
  const slices = requireArray(responseVariant(response, "SlicesListed", step).slices, `${step} slices`)
  const slice = slices.find((item) => item?.id === sliceId)
  if (!slice) return null
  const agentIds = requireArray(slice.agent_ids ?? [], `${step} agent_ids`)
    .map((agentId) => requireText(agentId, `${step} agent identity`))
  if (new Set(agentIds).size !== agentIds.length) throw new Error(`${step} returned duplicate agent identities`)
  return { slice, agentIds }
}

async function deleteOwnedRoom({ displayClient, requestApi, roomId, signal, step }) {
  if (!hasText(roomId)) return false
  const listSessions = requireRequestConstructor(requestApi, "listSessionsRequest")
  let sessions = await readSessionRecords({ displayClient, requestApi, signal, step: `${step} Room inventory` })
  let room = sessions.find((session) => session?.id === roomId)
  if (room) {
    if (room.status !== "ended") {
      const endResponse = await sendWithAbortSignal(
        displayClient,
        requireRequestConstructor(requestApi, "endSessionRequest")(roomId),
        signal,
        `${step} Room end`,
      )
      const ended = responseVariant(endResponse, "SessionEnded", `${step} Room end`).session
      if (ended?.id !== roomId) throw new Error(`${step} Room end returned a foreign Room identity`)
    }
    const deleteResponse = await sendWithAbortSignal(
      displayClient,
      requireRequestConstructor(requestApi, "deleteSessionRequest")(roomId),
      signal,
      `${step} Room delete`,
    )
    const deleted = responseVariant(deleteResponse, "SessionDeleted", `${step} Room delete`).session
    if (deleted?.id !== roomId) throw new Error(`${step} Room delete returned a foreign Room identity`)
  }
  await waitForKernelProbe(
    async () => {
      sessions = await readSessionRecords({ displayClient, requestApi, signal, step: `${step} Room deletion observation` })
      room = sessions.find((session) => session?.id === roomId)
      return !room
    },
    5_000,
    `${step} Room deletion`,
    signal,
  )
  return true
}

async function readSessionRecords({ displayClient, client = displayClient, requestApi, signal, step }) {
  const response = await sendWithAbortSignal(
    client,
    requireRequestConstructor(requestApi, "listSessionsRequest")(),
    signal,
    step,
  )
  return requireArray(responseVariant(response, "SessionsListed", step).sessions, `${step} sessions`)
}

async function detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step }) {
  if (ownedResources.attachmentIds.size === 0) return
  const unresolvedAttachmentIds = [...ownedResources.attachmentIds]
    .filter((attachmentId) => !ownedResources.detachedAttachmentIds.has(attachmentId))
  if (unresolvedAttachmentIds.length === 0) return

  const sessionsResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "listSessionsRequest")(),
    signal,
    `${step} attachment reconciliation`,
  )
  const sessions = requireArray(
    responseVariant(sessionsResponse, "SessionsListed", `${step} attachment reconciliation`).sessions,
    `${step} attachment reconciliation SessionsListed.sessions`,
  )
  const activeAttachmentIds = new Set()
  for (const session of sessions) {
    for (const attachmentId of session?.attachment_ids ?? session?.attachmentIds ?? []) {
      if (hasText(attachmentId)) activeAttachmentIds.add(attachmentId)
    }
  }
  for (const attachmentId of unresolvedAttachmentIds) {
    if (!activeAttachmentIds.has(attachmentId)) {
      ownedResources.detachedAttachmentIds.add(attachmentId)
    }
  }

  const detach = requireRequestConstructor(requestApi, "detachFromSessionRequest")
  for (const attachmentId of ownedResources.attachmentIds) {
    if (ownedResources.detachedAttachmentIds.has(attachmentId)) continue
    const response = await sendWithAbortSignal(
      displayClient,
      detach(attachmentId),
      signal,
      `${step} attachment detach`,
    )
    const detached = responseVariant(response, "SessionDetached", `${step} attachment detach`).attachment
    validateSessionAttachment(detached, attachmentId, null, `${step} attachment detach`)
    ownedResources.detachedAttachmentIds.add(attachmentId)
  }
}

function requireBinding(request, step) {
  const binding = request?.binding
  if (!binding || typeof binding !== "object") {
    throw new Error(`managed parity ${step} requires a target binding`)
  }
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
    requireText(binding[field], `${step} binding.${field}`)
  }
  return binding
}

function validateCreatedSlice(slice, { roomId, workerKernelRef, targetMachineRef, step }) {
  if (!slice || typeof slice !== "object") {
    throw new Error(`managed parity ${step} returned no created slice`)
  }
  requireText(slice.id, `${step} SliceCreated.slice.id`)
  if (slice.backend !== "ssh_docker") {
    throw new Error(`managed parity ${step} created a non-managed slice backend`)
  }
  if (slice.display_mode !== "headed") {
    throw new Error(`managed parity ${step} created a non-headed slice`)
  }
  if (slice.worker_kernel_ref !== workerKernelRef) {
    throw new Error(`managed parity ${step} created a slice for the wrong worker reference`)
  }
  if (slice.environment_session_id !== undefined
    && slice.environment_session_id !== null
    && slice.environment_session_id !== roomId) {
    throw new Error(`managed parity ${step} created a slice already owned by another Room`)
  }
  if (slice.session_id !== undefined && slice.session_id !== null && slice.session_id !== roomId) {
    throw new Error(`managed parity ${step} created a slice already attached to another Room`)
  }
  if (hasText(targetMachineRef) && slice.worker_machine_id !== undefined
    && slice.worker_machine_id !== null && slice.worker_machine_id !== targetMachineRef) {
    throw new Error(`managed parity ${step} created a slice for the wrong worker machine`)
  }
}

function observedDisplayBackend(slice, step, expected = "selkies") {
  const endpoint = slice?.display_endpoint
  if (!endpoint || typeof endpoint !== "object") {
    throw new Error(`managed parity ${step} returned no kernel-selected display endpoint`)
  }
  if (endpoint.slice_id !== slice.id || endpoint.kind !== expected) {
    throw new Error(`managed parity ${step} did not return the kernel-selected ${expected} backend`)
  }
  return endpoint.kind
}

function validateRoomSliceBinding(binding, { roomId, sliceId, workerKernelRef }) {
  if (!binding || typeof binding !== "object") {
    throw new Error("managed parity selkies.create Room binding returned no binding")
  }
  if (binding.session_id !== roomId || binding.slice_id !== sliceId) {
    throw new Error("managed parity selkies.create Room binding returned the wrong Room or slice")
  }
  requireText(binding.owner_kernel_id, "RoomEnvironmentSlice.binding.owner_kernel_id")
  if (binding.worker_kernel_ref !== workerKernelRef) {
    throw new Error("managed parity selkies.create Room binding returned the wrong worker reference")
  }
}

function validateStartedSlice(slice, {
  sliceId,
  roomId,
  workerKernelRef,
  binding,
  targetMachineRef,
  step,
}) {
  if (!slice || typeof slice !== "object") {
    throw new Error(`managed parity ${step} returned no started slice`)
  }
  if (slice.id !== sliceId || slice.status !== "running") {
    throw new Error(`managed parity ${step} did not return the running created slice`)
  }
  if (slice.worker_kernel_ref !== workerKernelRef) {
    throw new Error(`managed parity ${step} returned the wrong worker reference`)
  }
  if (slice.environment_session_id !== undefined
    && slice.environment_session_id !== null && slice.environment_session_id !== roomId) {
    throw new Error(`managed parity ${step} returned a slice bound to the wrong Room`)
  }
  const workerKernelId = requireText(slice.worker_kernel_id, `${step} slice.worker_kernel_id`)
  const workerMachineId = requireText(slice.worker_machine_id, `${step} slice.worker_machine_id`)
  if (workerKernelId !== binding.kernelId || workerMachineId !== binding.machineId) {
    throw new Error(`managed parity ${step} returned worker identities different from the target binding`)
  }
  if (hasText(targetMachineRef) && workerMachineId !== targetMachineRef) {
    throw new Error(`managed parity ${step} returned a worker machine different from configured target`)
  }
}

function validateSessionAttachment(attachment, expectedId, roomId, step) {
  if (!attachment || typeof attachment !== "object") {
    throw new Error(`managed parity ${step} returned no attachment`)
  }
  const attachmentId = requireText(attachment.id, `${step} attachment.id`)
  if (expectedId && attachmentId !== expectedId) {
    throw new Error(`managed parity ${step} returned a different attachment identity`)
  }
  if (roomId !== null && attachment.session_id !== roomId) {
    throw new Error(`managed parity ${step} attached a different Room`)
  }
  return attachmentId
}

function validateDeletedSlice(slice, sliceId, step) {
  if (!slice || typeof slice !== "object" || slice.id !== sliceId) {
    throw new Error(`managed parity ${step} returned a different deleted slice identity`)
  }
}

async function observeAuthoritativeResourceCounts({
  displayClient,
  requestApi,
  roomId,
  environmentId,
  sliceId,
  signal,
  step,
}) {
  // The home kernel's public inventories are the authority for these counts.
  // A Room-bound SliceRecord is the durable reservation for one physical
  // browser/profile. Filter to this Room; never count the shared host or
  // assume `1`.
  const sessionsResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "listSessionsRequest")(),
    signal,
    `${step} Room inventory`,
  )
  const sessions = requireArray(
    responseVariant(sessionsResponse, "SessionsListed", `${step} Room inventory`).sessions,
    `${step} SessionsListed.sessions`,
  )
  const slicesResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "listSlicesRequest")(),
    signal,
    `${step} browser/profile inventory`,
  )
  const slices = requireArray(
    responseVariant(slicesResponse, "SlicesListed", `${step} browser/profile inventory`).slices,
    `${step} SlicesListed.slices`,
  )
  const roomCount = sessions.filter((session) => session?.id === roomId).length
  const environmentSlices = slices.filter(
    (slice) => slice?.environment_session_id === roomId,
  )
  if (!environmentSlices.some((slice) => slice?.id === sliceId)) {
    throw new Error(`${step} inventory did not return the created environment slice`)
  }
  const getResourceInventoryRequest = requireRequestConstructor(
    requestApi,
    "getRoomEnvironmentResourceInventoryRequest",
  )
  const inventoryResponse = await sendWithAbortSignal(
    displayClient,
    getResourceInventoryRequest(roomId, sliceId),
    signal,
    `${step} worker resource inventory`,
  )
  const inventory = responseVariant(
    inventoryResponse,
    "RoomEnvironmentResourceInventory",
    `${step} worker resource inventory`,
  ).inventory
  if (!inventory || typeof inventory !== "object"
    || inventory.session_id !== roomId
    || inventory.environment_id !== environmentId
    || inventory.slice_id !== sliceId) {
    throw new Error(`${step} worker resource inventory returned the wrong Room, Environment, or slice`)
  }
  const browserIds = requireUniqueIdentityArray(
    inventory.browser_ids,
    `${step} worker resource inventory.browser_ids`,
  )
  const profileIds = requireUniqueIdentityArray(
    inventory.profile_ids,
    `${step} worker resource inventory.profile_ids`,
  )
  if (browserIds.length !== 1 || profileIds.length !== 1) {
    throw new Error(
      `${step} worker resource inventory requires exactly one worker browser and profile identity`,
    )
  }
  return {
    roomCount,
    browserCount: browserIds.length,
    profileCount: profileIds.length,
  }
}

function requireUniqueIdentityArray(value, label) {
  const values = requireArray(value, label)
  const ids = values.map((value) => requireText(value, `${label} identity`).trim())
  if (new Set(ids).size !== ids.length) {
    throw new Error(`managed parity response requires unique ${label} identities`)
  }
  return ids
}

function resolveOwnedSliceId(request, ownedResources, step, { requireOwned = false } = {}) {
  if (requireOwned && !ownedResources.sliceId) {
    throw new Error(`managed parity ${step} requires a tracked owned slice before deletion`)
  }
  const requestedSliceId = hasText(request?.sliceId) ? request.sliceId.trim() : null
  if (requestedSliceId && ownedResources.sliceId && requestedSliceId !== ownedResources.sliceId) {
    throw new Error(`managed parity ${step} rejected a stale slice identity`)
  }
  return requireText(requestedSliceId ?? ownedResources.sliceId, `${step} sliceId`)
}

function requireRequestConstructor(requestApi, name) {
  if (!requestApi || typeof requestApi[name] !== "function") {
    throw new Error(`managed parity requires released kernel request constructor ${name}`)
  }
  return requestApi[name]
}

function clearOwnedResources(ownedResources) {
  ownedResources.sliceId = null
  ownedResources.attachmentIds.clear()
  ownedResources.attachmentsByClient.clear()
  ownedResources.agentIds.clear()
  ownedResources.identity = null
}

function rememberCleanupEvidence(ownedResources, evidence) {
  const previous = ownedResources.cleanupEvidence
  ownedResources.cleanupEvidence = redactManagedValue({
    schema: MANAGED_PARITY_SCHEMA,
    cleaned: evidence.cleaned === true,
    sliceId: evidence.sliceId ?? previous?.sliceId ?? ownedResources.stableIdentity?.sliceId ?? null,
    attachmentIds: [...new Set(evidence.attachmentIds ?? previous?.attachmentIds ?? [])],
    detachedAttachmentIds: [...new Set([
      ...(previous?.detachedAttachmentIds ?? []),
      ...ownedResources.detachedAttachmentIds,
    ])],
    agentIds: [...new Set(evidence.agentIds ?? previous?.agentIds ?? [...ownedResources.agentIds])],
    identity: evidence.identity ?? previous?.identity ?? ownedResources.stableIdentity ?? null,
    deleted: evidence.deleted === true,
    roomDeleted: evidence.roomDeleted === true || previous?.roomDeleted === true,
    postDelete: evidence.postDelete ?? previous?.postDelete ?? ownedResources.cleanupMeasurements?.postDelete ?? null,
    measurements: evidence.measurements ?? previous?.measurements ?? ownedResources.cleanupMeasurements ?? null,
    reason: evidence.reason ?? "cleanup",
  })
}

async function runSelkiesAttach({
  displayClient,
  identityClient,
  requestApi,
  displayTransport,
  ownedResources,
  request,
  signal,
}) {
  const binding = request?.binding
  if (!binding || typeof binding !== "object") {
    throw new Error("managed parity selkies.attach requires a target binding")
  }
  if (request.displayBackend !== "selkies") {
    throw new Error("managed parity selkies.attach requires the selkies display backend")
  }
  if (!["web", "local_tui", "remote_tui"].includes(request.client)) {
    throw new Error("managed parity selkies.attach requires a documented client")
  }

  if (!displayTransport?.openSelkiesDisplayStream
    && !hasText(request.sliceId)
    && !ownedResources.sliceId
    && !hasText(request.attachmentId)) {
    throw new Error(
      "managed parity selkies.attach requires sliceId, attachmentId, and viewerPublicKey for public display authorization",
    )
  }

  const roomId = requireText(binding.roomId, "binding.roomId")
  const sliceId = resolveOwnedSliceId(request, ownedResources, "selkies.attach")
  const identity = displayTransport?.openSelkiesDisplayStream
    ? await readAuthoritativeBinding({
      displayClient,
      identityClient,
      requestApi,
      roomId,
      signal,
      step: "selkies.attach",
    })
    : null
  if (identity) assertBinding(identity, binding, "selkies.attach")

  const { attachmentId } = await resolveAttachment({
    displayClient,
    requestApi,
    ownedResources,
    request,
    roomId,
    signal,
  })

  if (displayTransport?.openSelkiesDisplayStream) {
    let stream
    try {
      stream = await displayTransport.openSelkiesDisplayStream({
        client: displayClient,
        sliceId,
        sessionId: roomId,
        attachmentId,
        webSocket: displayTransport.webSocket,
        signal,
        connectTimeoutMs: request.connectTimeoutMs ?? DISPLAY_CONNECT_TIMEOUT_MS,
      })
      await stream.sendControl("START_VIDEO", { signal })
      const readyDeadline = Date.now() + (request.streamReadyTimeoutMs ?? DISPLAY_READY_TIMEOUT_MS)
      const startupMessage = await receiveDisplayMessageUntil(
        stream,
        signal,
        readyDeadline,
        (message) => message.kind === "text" && new TextDecoder().decode(message.data) === "VIDEO_STARTED",
        "VIDEO_STARTED",
      )
      const firstFrame = await receiveDisplayMessageUntil(
        stream,
        signal,
        readyDeadline,
        (message) => message.kind === "binary"
          && message.data.byteLength > 10
          && message.data.byteLength <= 4 * 1024 * 1024
          && message.data[0] === 4,
        "a valid Selkies video frame",
      )
      return {
        ...identity,
        client: request.client,
        displayBackend: request.displayBackend,
        attached: true,
        sliceId,
        attachmentId,
        displayProtocol: stream.endpoint.stream_protocol,
        displayStreamId: stream.endpoint.stream_id,
        startupMessage: {
          kind: startupMessage.kind,
          byteLength: startupMessage.data.byteLength,
        },
        firstFrame: {
          kind: firstFrame.kind,
          byteLength: firstFrame.data.byteLength,
          recordType: firstFrame.data[0],
        },
      }
    } finally {
      await stream?.close()
    }
  }

  if (!hasText(request.viewerPublicKey)) {
    throw new Error(
      "managed parity selkies.attach requires viewerPublicKey for public display authorization",
    )
  }
  const viewerPublicKey = request.viewerPublicKey.trim()
  const displayEndpointResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.getSliceDisplayEndpointRequest(sliceId, {
      sessionId: roomId,
      attachmentId,
      viewerPublicKey,
    }),
    signal,
    "selkies.attach display authorization",
  )
  const endpoint = responseVariant(
    displayEndpointResponse,
    "SliceDisplayEndpoint",
    "selkies.attach display authorization",
  ).endpoint
  if (!endpoint || typeof endpoint !== "object") {
    throw new Error("managed parity selkies.attach display authorization returned no endpoint")
  }
  if (endpoint.slice_id !== sliceId || endpoint.kind !== "selkies") {
    throw new Error("managed parity selkies.attach display authorization returned the wrong Selkies endpoint")
  }
  requireText(endpoint.url, "SliceDisplayEndpoint.endpoint.url")
  requireText(endpoint.access, "SliceDisplayEndpoint.endpoint.access")
  requireText(endpoint.stream_protocol, "SliceDisplayEndpoint.endpoint.stream_protocol")
  requireText(endpoint.stream_id, "SliceDisplayEndpoint.endpoint.stream_id")
  requireText(endpoint.peer_public_key, "SliceDisplayEndpoint.endpoint.peer_public_key")
  throw new Error(
    "managed parity selkies.attach has no public Selkies display-stream connection API after authorization",
  )
}

async function runNovncAttach({
  displayClient,
  identityClient,
  requestApi,
  ownedResources,
  request,
  signal,
}) {
  const binding = requireBinding(request, "novnc.attach")
  if (request.displayBackend !== "novnc") {
    throw new Error("managed parity novnc.attach requires the novnc display backend")
  }
  if (!["web", "local_tui", "remote_tui"].includes(request.client)) {
    throw new Error("managed parity novnc.attach requires a documented client")
  }
  const roomId = requireText(binding.roomId, "novnc.attach binding.roomId")
  const sliceId = resolveOwnedSliceId(request, ownedResources, "novnc.attach")
  const identity = await readAuthoritativeBinding({
    displayClient,
    identityClient,
    requestApi,
    roomId,
    signal,
    step: "novnc.attach",
  })
  assertBinding(identity, binding, "novnc.attach")
  const { attachmentId } = await resolveAttachment({
    displayClient,
    requestApi,
    ownedResources,
    request,
    roomId,
    signal,
  })
  return {
    ...identity,
    client: request.client,
    displayBackend: "novnc",
    attached: true,
    sliceId,
    attachmentId,
  }
}

async function runNovncRollback({
  displayClient,
  requestApi,
  ownedResources,
  request,
  signal,
}) {
  const binding = requireBinding(request, "novnc.rollback")
  if (request.displayBackend !== "novnc") {
    throw new Error("managed parity novnc.rollback requires the novnc display backend")
  }
  const roomId = requireText(binding.roomId, "novnc.rollback binding.roomId")
  if (typeof requestApi.retryRoomEnvironmentRequest !== "function") {
    throw new Error("managed parity novnc.rollback requires the released Room retry request")
  }
  const retryResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.retryRoomEnvironmentRequest(roomId),
    signal,
    "novnc.rollback",
  )
  const environment = responseVariant(retryResponse, "RoomEnvironmentUpdated", "novnc.rollback").environment
  if (environment?.session_id !== roomId || environment?.environment_id !== binding.environmentId) {
    throw new Error("managed parity novnc.rollback returned the wrong Room environment")
  }
  if (ownedResources.sliceId && typeof requestApi.getSliceRequest === "function") {
    const sliceResponse = await sendWithAbortSignal(
      displayClient,
      requestApi.getSliceRequest(ownedResources.sliceId),
      signal,
      "novnc.rollback slice identity",
    )
    const slice = responseVariant(sliceResponse, "Slice", "novnc.rollback slice identity").slice
    if (slice?.id !== ownedResources.sliceId) {
      throw new Error("managed parity novnc.rollback lost the owned slice identity")
    }
  }
  return {
    ...binding,
    displayBackend: "novnc",
    rollbackReachable: true,
    finalAcceptance: false,
  }
}

async function resolveAttachment({
  displayClient,
  requestApi,
  ownedResources,
  request,
  roomId,
  signal,
}) {
  const requestedAttachmentId = hasText(request.attachmentId) ? request.attachmentId.trim() : null
  const clientKey = `${roomId}:${request.client}`
  const knownAttachmentId = ownedResources.attachmentsByClient.get(clientKey)
  if (requestedAttachmentId) {
    if (knownAttachmentId && knownAttachmentId !== requestedAttachmentId) {
      throw new Error("managed parity selkies.attach rejected a stale attachment identity")
    }
    return { attachmentId: requestedAttachmentId, owned: false }
  }
  if (knownAttachmentId) return { attachmentId: knownAttachmentId, owned: true }

  const attach = requireRequestConstructor(requestApi, "attachToSessionRequest")
  const runId = requireText(request.runId, "runId")
  const response = await sendWithAbortSignal(
    displayClient,
    attach(roomId, `managed-parity-${request.client}-${runId}`),
    signal,
    "selkies.attach Room attachment",
  )
  const attachment = responseVariant(
    response,
    "SessionAttached",
    "selkies.attach Room attachment",
  ).attachment
  const attachmentId = validateSessionAttachment(
    attachment,
    null,
    roomId,
    "selkies.attach Room attachment",
  )
  ownedResources.attachmentIds.add(attachmentId)
  ownedResources.attachmentsByClient.set(clientKey, attachmentId)
  return { attachmentId, owned: true }
}

async function receiveDisplayMessageUntil(stream, signal, deadline, matches, description) {
  while (true) {
    const remainingMs = deadline - Date.now()
    if (remainingMs <= 0) {
      throw new Error(`managed parity selkies.attach did not receive ${description} before the deadline`)
    }
    const message = await stream.receive({ signal, timeoutMs: remainingMs })
    if (matches(message)) return message
  }
}

async function readAuthoritativeBinding({
  displayClient,
  identityClient,
  requestApi,
  roomId,
  signal,
  step = "selkies.attach",
}) {
  const relayResponse = await sendWithAbortSignal(
    identityClient,
    requestApi.relayStatusRequest(),
    signal,
    `${step} relay identity`,
  )
  const relayStatus = responseVariant(relayResponse, "RelayStatus", `${step} relay identity`).status
  if (relayStatus?.configured !== true || relayStatus.connected !== true) {
    throw new Error(`managed parity ${step} requires a connected configured relay`)
  }
  const environmentResponse = await sendWithAbortSignal(
    displayClient,
    requestApi.getRoomEnvironmentStateRequest(roomId),
    signal,
    `${step} Room state`,
  )
  const environment = responseVariant(
    environmentResponse,
    "RoomEnvironmentState",
    `${step} Room state`,
  ).environment
  return {
    kernelId: requireText(relayStatus?.daemon_id, `${step} RelayStatus.status.daemon_id`),
    machineId: requireText(relayStatus?.machine_id, `${step} RelayStatus.status.machine_id`),
    roomId: requireText(environment?.session_id, `${step} RoomEnvironmentState.environment.session_id`),
    environmentId: requireText(environment?.environment_id, `${step} RoomEnvironmentState.environment.environment_id`),
  }
}

function assertBinding(value, binding, step) {
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
    if (value[field] !== binding[field]) {
      throw new Error(`managed parity target identity mismatch for ${step}: ${field}`)
    }
  }
}

async function sendWithAbortSignal(
  client,
  request,
  signal,
  step,
  timeoutMs = PUBLIC_REQUEST_TIMEOUT_MS,
) {
  return withDeadline(
    () => client.send(request),
    {
      signal,
      timeoutMs,
      step,
    },
  ).catch((error) => {
    if (error?.name === "AbortError" || /timed out/.test(error?.message ?? "")) {
      closeAfterAbort(client)
    }
    throw error
  })
}

function abortError(step) {
  const error = new Error(`managed parity ${step} was aborted while the public request was in flight`)
  error.name = "AbortError"
  return error
}

function closeAfterAbort(client) {
  const close = typeof client.close === "function"
    ? client.close
    : typeof client.destroy === "function"
      ? client.destroy
      : null
  if (!close) return
  try {
    Promise.resolve(close.call(client)).catch(() => {})
  } catch {
    // Abort must reject the public operation even if transport teardown fails.
  }
}

function resolvedConnection(response) {
  const connection = responseVariant(response, "KernelClientConnectionResolved", "resolve target").connection
  if (!connection || typeof connection !== "object") {
    throw new Error("managed parity target connection response was malformed")
  }
  return connection
}

function responseVariant(response, variant, step) {
  if (!response || typeof response !== "object" || !(variant in response)) {
    throw new Error(`managed parity ${step} returned an unexpected response; expected ${variant}`)
  }
  return response[variant]
}

function responseVariantAny(response, variants, step) {
  for (const variant of variants) {
    if (response && typeof response === "object" && variant in response) return response[variant]
  }
  throw new Error(`managed parity ${step} returned an unexpected response; expected one of ${variants.join(", ")}`)
}

function requiredOperatorValue(name) {
  const value = process.env[name]?.trim()
  if (!value) throw new Error(`${name} is required`)
  return value
}

function requireStandardLocalAuthConfiguration() {
  if (process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN === undefined
    && process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE === undefined) {
    throw new Error(
      "managed parity requires CHARIOX_KERNEL_LOCAL_AUTH_TOKEN or CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE",
    )
  }
}

function requireText(value, label) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`managed parity response requires ${label}`)
  }
  return value
}

function requireArray(value, label) {
  if (!Array.isArray(value)) {
    throw new Error(`managed parity response requires ${label}`)
  }
  return value
}

function hasText(value) {
  return typeof value === "string" && value.trim() !== ""
}
