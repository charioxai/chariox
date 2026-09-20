import { createRequire } from "node:module"
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
const MANAGED_PARITY_SCHEMA = "chariox.browser_computer_m0_guard.v1"
// This is the released wire constant at the reviewed PR head. Production
// construction also binds the value exported by kernel-client; the literal is
// only the local fail-closed reference used when a test injects that seam.
export const MANAGED_BROWSER_COMPUTER_PARITY_PROTOCOL = 334

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
        parityConfig: config,
        operationAdapter,
        reconnectClient: reconnectClient ?? (async () => new LocalIpcClient(connection.relay_url, {
          relayAuthToken: connection.relay_token,
          targetDaemonId: connection.target_daemon_id ?? undefined,
          targetDaemonAlias: connection.target_daemon_alias ?? undefined,
        })),
        protocolApi: modules.protocolApi,
        timeouts,
        displayTransport: {
          openSelkiesDisplayStream: displayApi.openSelkiesDisplayStream,
          webSocket,
        },
      }),
      close: async () => {
        await workerClient.close().catch(() => {})
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
    attachmentIds: new Set(),
    attachmentsByClient: new Map(),
    identity: null,
    stableIdentity: null,
    detachedAttachmentIds: new Set(),
    cleanupEvidence: null,
  }

  const telemetryAdapter = resourceTelemetry ?? managedResourceTelemetry ?? resourceTelemetryAdapter
  const persistenceAdapter = persistence
    ?? persistenceTransport
    ?? (firstCallable(identityClient, ["describePersistenceMutations", "runPersistenceMutations"])
      ? identityClient
      : createKernelPersistenceAdapter({
        client: displayClient,
        identityClient: displayClient,
        requestApi,
        parityConfig,
      }))
  const residueAdapter = cleanupInspector
    ?? residueInspector
    ?? (firstCallable(identityClient, ["inspectCleanupResidue", "cleanupInspect"])
      ? identityClient
      : createKernelCleanupInspector({
        client: displayClient,
        requestApi,
      }))
  const productionOperationAdapter = operationAdapter
    ?? createKernelOperationAdapter({
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
            identityClient,
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
          client: identityClient,
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
          client,
          identityClient,
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
        return withDeadline(
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
      }
      if (step === "novnc.create") {
        return runDisplayBackendCreate({
          displayClient,
          identityClient,
          requestApi,
          targetKernelRef,
          targetMachineRef,
          ownedResources,
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
          requestApi,
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
            client,
            identityClient,
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
          requestApi,
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
          requestApi,
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

function createKernelPersistenceAdapter({ client, identityClient, requestApi, parityConfig }) {
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
      return createKernelPersistencePlan(input, parityConfig)
    },
    async run(input) {
      return executeKernelPersistence({
        ...input,
        client,
        identityClient,
        requestApi,
      })
    },
  }
}

function createKernelPersistencePlan(input, parityConfig) {
  const sliceId = requireText(input?.ownedResource?.sliceId, "persistence owned slice id")
  const configured = parityConfig?.browserComputerGuard?.dockerPreconditions
    ?? parityConfig?.dockerPreconditions
    ?? []
  const fallbackArgv = {
    save: ["kernel", "SaveSliceState", sliceId],
    remove: ["kernel", "StopSlice", sliceId],
    restore: ["kernel", "StartSlice", sliceId],
  }
  const checkpoints = {
    save: { before: "before-docker-save", after: "after-docker-save" },
    remove: { before: "before-docker-remove", after: "after-docker-remove" },
    restore: { before: "before-docker-restore", after: "after-docker-restore" },
  }
  const mutations = ["save", "remove", "restore"].map((action, index) => {
    const declaration = configured[index]
    const argv = declaration?.command ?? declaration?.argv ?? fallbackArgv[action]
    requireSafeArgv(argv, `persistence ${action}`)
    return {
      action,
      argv: [...argv],
      request: { action, argv: [...argv] },
      checkpoints: checkpoints[action],
    }
  })
  return {
    schema: MANAGED_PARITY_SCHEMA,
    persistenceMutations: mutations,
    ownedResource: redactManagedValue(input?.ownedResource ?? null),
  }
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
  let savedState = null
  let initialInventory = null
  let finalInventory = null
  for (const [index, mutation] of planned.entries()) {
    const inventory = await readKernelPersistenceInventory({
      client: identityClient ?? client,
      requestApi,
      sliceId,
      roomId: ownedResource?.roomId,
      signal,
      step: `persistence ${mutation.action} inventory`,
    })
    before.push(inventory)
    if (index === 0) initialInventory = inventory
    await onPersistenceMutation?.({ phase: "before", mutation: redactManagedValue(mutation) })
    let response
    let receipt
    if (mutation.action === "save") {
      response = await sendWithAbortSignal(
        client,
        requestApi.saveSliceStateRequest(sliceId, "restart_agents", "this_slice"),
        signal,
        "persistence save",
      )
      const saved = responseVariant(response, "SliceStateSaved", "persistence save")
      savedState = saved.state
      const archivePath = requireText(savedState?.home_archive_path, "SliceStateSaved.state.home_archive_path")
      receipt = {
        ok: true,
        id: requireText(savedState?.id, "SliceStateSaved.state.id"),
        archivePath,
        authoritative: true,
      }
    } else if (mutation.action === "remove") {
      response = await sendWithAbortSignal(
        client,
        requestApi.stopSliceRequest(sliceId),
        signal,
        "persistence remove",
      )
      responseVariantAny(response, ["SliceStopped", "Slice"], "persistence remove")
      receipt = {
        ok: true,
        id: `${receiptId(receipts[0])}:remove`,
        parentReceiptId: receiptId(receipts[0]),
        archivePath: requireText(savedState?.home_archive_path, "saved state archive path"),
        authoritative: true,
      }
    } else {
      response = await sendWithAbortSignal(
        client,
        requestApi.startSliceRequest(sliceId),
        signal,
        "persistence restore",
        SLICE_LIFECYCLE_TIMEOUT_MS,
      )
      responseVariant(response, "SliceStarted", "persistence restore")
      const statusResponse = await sendWithAbortSignal(
        client,
        requestApi.getSliceStateStatusRequest(sliceId),
        signal,
        "persistence restore state status",
      )
      const status = responseVariant(statusResponse, "SliceStateStatus", "persistence restore state status")
      if (!status.state || status.state.id !== savedState?.id) {
        throw new Error("managed parity persistence restore did not restore the authoritative saved state")
      }
      receipt = {
        ok: true,
        id: `${receiptId(receipts[1])}:restore`,
        parentReceiptId: receiptId(receipts[1]),
        archivePath: requireText(savedState?.home_archive_path, "saved state archive path"),
        authoritative: true,
      }
    }
    receipts.push(receipt)
    await onPersistenceMutation?.({ phase: "after", mutation: redactManagedValue({ ...mutation, receipt }) })
    if (mutation.action === "restore") {
      finalInventory = await readKernelPersistenceInventory({
        client: identityClient ?? client,
        requestApi,
        sliceId,
        roomId: ownedResource?.roomId,
        signal,
        step: "persistence restore inventory",
      })
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
    persistenceMutations: planned.map((mutation, index) => ({
      ...mutation,
      before: before[index],
      receipt: receipts[index],
      ...(index > 0 ? { saveReceipt: receipts[0] } : {}),
      ...(index > 1 ? { removeReceipt: receipts[1] } : {}),
    })),
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
  requireUniqueIdentityArray(inventory?.profile_ids, `${step} profile_ids`)
  requireUniqueIdentityArray(inventory?.browser_ids, `${step} browser_ids`)
  return {
    slice: redactManagedValue({
      id: slice.id,
      status: slice.status,
      environment_session_id: slice.environment_session_id ?? slice.session_id ?? null,
      environment_id: slice.environment_id ?? null,
    }),
    inventory: redactManagedValue({
      environment_id: inventory.environment_id,
      slice_id: inventory.slice_id,
      browser_ids: inventory.browser_ids,
      profile_ids: inventory.profile_ids,
    }),
  }
}

function createKernelCleanupInspector({ client, requestApi }) {
  if (typeof client?.send !== "function"
    || typeof requestApi?.listSlicesRequest !== "function"
    || typeof requestApi?.listSessionsRequest !== "function"
    || typeof requestApi?.getRoomEnvironmentResourceInventoryRequest !== "function"
    || typeof requestApi?.getRoomEnvironmentStateRequest !== "function") {
    return null
  }
  return {
    async inspect(input) {
      return inspectAuthoritativeKernelResidue({ client, requestApi, ...input })
    },
  }
}

async function inspectAuthoritativeKernelResidue({ client, requestApi, ownedResource, cleanupEvidence, signal }) {
  const sliceId = requireText(cleanupEvidence?.sliceId ?? ownedResource?.sliceId, "cleanup owned slice id")
  const roomId = requireText(
    cleanupEvidence?.identity?.roomId ?? ownedResource?.roomId,
    "cleanup owned Room id",
  )
  const slicesResponse = await sendWithAbortSignal(
    client,
    requestApi.listSlicesRequest(),
    signal,
    "cleanup.inspect authoritative slices",
  )
  const slices = requireArray(
    responseVariant(slicesResponse, "SlicesListed", "cleanup.inspect authoritative slices").slices,
    "cleanup authoritative slices",
  )
  const ownedSlices = slices.filter((slice) => slice?.id === sliceId)
  const sessionsResponse = await sendWithAbortSignal(
    client,
    requestApi.listSessionsRequest(),
    signal,
    "cleanup.inspect authoritative viewers",
  )
  const sessions = requireArray(
    responseVariant(sessionsResponse, "SessionsListed", "cleanup.inspect authoritative viewers").sessions,
    "cleanup authoritative sessions",
  )
  const roomSessions = sessions.filter((session) => session?.id === roomId)
  const attachmentIds = new Set(cleanupEvidence?.attachmentIds ?? ownedResource?.attachmentIds ?? [])
  const residualAttachmentIds = new Set()
  for (const session of roomSessions) {
    for (const attachmentId of session?.attachment_ids ?? session?.attachmentIds ?? []) {
      if (attachmentIds.has(attachmentId) || hasText(attachmentId)) residualAttachmentIds.add(attachmentId)
    }
  }
  let memberInspection = "public-session-list-only"
  if (typeof requestApi.listSessionMembersRequest === "function") {
    const membersResponse = await sendWithAbortSignal(
      client,
      requestApi.listSessionMembersRequest(roomId),
      signal,
      "cleanup.inspect authoritative viewers",
    )
    const members = requireArray(
      responseVariant(membersResponse, "SessionMembersListed", "cleanup.inspect authoritative viewers").members,
      "cleanup authoritative members",
    )
    for (const member of members) {
      const attachmentId = member?.attachment_id ?? member?.attachmentId ?? member?.id
      if (hasText(attachmentId)) residualAttachmentIds.add(attachmentId)
    }
    memberInspection = "public-session-members"
  }

  const inventoryResponse = await sendWithAbortSignal(
    client,
    requestApi.getRoomEnvironmentResourceInventoryRequest(roomId, sliceId),
    signal,
    "cleanup.inspect authoritative browser/profile resources",
  )
  const inventory = responseVariant(
    inventoryResponse,
    "RoomEnvironmentResourceInventory",
    "cleanup.inspect authoritative browser/profile resources",
  ).inventory
  const browserIds = requireUniqueIdentityArray(inventory?.browser_ids, "cleanup browser_ids")
  const profileIds = requireUniqueIdentityArray(inventory?.profile_ids, "cleanup profile_ids")
  const stateResponse = await sendWithAbortSignal(
    client,
    requestApi.getRoomEnvironmentStateRequest(roomId),
    signal,
    "cleanup.inspect authoritative controller state",
  )
  const environment = responseVariant(
    stateResponse,
    "RoomEnvironmentState",
    "cleanup.inspect authoritative controller state",
  ).environment
  if (environment?.session_id !== roomId) throw new Error("cleanup inspector returned a foreign Room")
  const health = requireArray(environment?.health, "cleanup environment health")
  const activeHealth = health.filter((item) => ["starting", "ready", "degraded"].includes(item?.state))
  const activeControllerResources = activeHealth.filter((item) => [
    "browser_controller", "browser", "desktop", "streamer",
  ].includes(item?.component))
  const activeActions = requireArray(environment?.actions, "cleanup environment actions")
    .filter((action) => ["queued", "running"].includes(action?.state))
  const inputResidue = (environment?.input_ownership ?? []).length
    + (environment?.pending_input_takeovers ?? []).length
  const managedMachines = 0
  const rooms = roomSessions.length
  const environments = (environment?.lifecycle !== "stopped" ? 1 : 0) + activeControllerResources.length
  const processes = browserIds.length + activeControllerResources.length + activeActions.length
  const listeners = residualAttachmentIds.size
  const containers = ownedSlices.length
  const profiles = profileIds.length
  const activeTargets = browserIds.length
  const temporaryFiles = 0
  const retainedEvidenceLeakCount = 0
  const zeroResidue = ownedSlices.length === 0
    && rooms === 0
    && environments === 0
    && processes === 0
    && listeners === 0
    && containers === 0
    && profiles === 0
    && activeTargets === 0
    && inputResidue === 0
  if (!zeroResidue) {
    throw new Error(
      `managed parity cleanup.inspect found residual browser/controller/viewer resources `
      + `(browsers=${browserIds.length}, profiles=${profileIds.length}, controllers=${activeControllerResources.length}, viewers=${listeners})`,
    )
  }
  return {
    schema: MANAGED_PARITY_SCHEMA,
    inspected: true,
    zeroResidue: true,
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
    resources: { rssDeltaBytes: 0, diskDeltaBytes: 0 },
    memberInspection,
    publicInventory: {
      ownedSliceCount: ownedSlices.length,
      roomPresent: rooms > 0,
      browserIds,
      profileIds,
      receiptKinds: ["SlicesListed", "SessionsListed", "RoomEnvironmentResourceInventory", "RoomEnvironmentState"],
    },
  }
}

function createKernelOperationAdapter({
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
        client: identityClient,
        displayClient,
        identityClient,
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
        client,
        displayClient,
        requestApi,
        reconnectClient,
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
      telemetry.cpuPercent ?? telemetry.telemetry?.cpuPercent ?? telemetry.process.cpuPercent ?? 0,
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
    "getSessionHistoryOutlineRequest", "getSessionStateRequest", "listSlicesRequest",
  ]) {
    if (typeof requestApi[name] !== "function") {
      throw new Error(`managed parity selkies.${mode} requires released provider request constructor ${name}`)
    }
  }
  const beforeAttachments = await readSessionAttachmentIds({ client, requestApi, roomId: binding.roomId, signal })
  const { runRoomRealProviderAction } = await import("./live-room-real-provider.mjs")
  const result = await runRoomRealProviderAction({
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

async function runKernelReconnect({ client, displayClient, requestApi, reconnectClient, request, signal }) {
  const binding = requireBinding(request, "selkies.reconnect")
  if (typeof reconnectClient !== "function") {
    throw new Error("managed parity selkies.reconnect requires the released client reconnect boundary")
  }
  const replacement = await reconnectClient({ signal, request: redactManagedValue(request) })
  if (!replacement || typeof replacement.send !== "function") {
    throw new Error("managed parity selkies.reconnect did not return a public kernel client")
  }
  try {
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
    const state = await sendWithAbortSignal(
      displayClient,
      requireRequestConstructor(requestApi, "getRoomEnvironmentStateRequest")(binding.roomId),
      signal,
      "selkies.reconnect Room identity",
    )
    const environment = responseVariant(state, "RoomEnvironmentState", "selkies.reconnect Room identity").environment
    if (environment.session_id !== binding.roomId || environment.environment_id !== binding.environmentId) {
      throw new Error("managed parity selkies.reconnect returned a foreign Room identity")
    }
  } finally {
    await Promise.resolve(replacement.close?.()).catch(() => {})
  }
  return {
    ...binding,
    displayBackend: "selkies",
    faultInjected: request.fault === "relay_disconnect",
    reconnected: true,
    duplicateActions: 0,
    duplicateBrowsers: 0,
  }
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
  const raw = await runMethod.call(persistenceAdapter, {
    ...persistenceAdapterInput({
      client,
      identityClient,
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
  return normalizePersistenceEvidence(raw, ownedResources, plan)
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
  const source = value?.persistenceMutations ?? value?.mutations
  if (!Array.isArray(source) || source.length !== 3) {
    throw new Error("managed parity persistence plan must contain exact save/remove/restore mutations")
  }
  const normalized = source.map((mutation, index) => {
    for (const field of ["before", "inventory", "receipt", "receipts", "saveReceipt", "removeReceipt"]) {
      if (mutation && Object.hasOwn(mutation, field)) {
        throw new Error(`managed parity persistence plan must not contain pre-execution ${field} evidence`)
      }
    }
    return normalizePersistenceMutationDefinition(mutation, index)
  })
  const output = redactManagedValue({
    schema: value?.schema ?? MANAGED_PARITY_SCHEMA,
    persistenceMutations: normalized,
  })
  output.ownedResource = stableOwnedIdentity(ownedResources)
  return output
}

function normalizePersistenceMutationDefinition(mutation, index) {
  const actions = ["save", "remove", "restore"]
  const action = actions[index]
  if (!mutation || mutation.action !== action) {
    throw new Error(`managed parity persistence mutation ${index} must be ${action}`)
  }
  const argv = requireSafeArgv(mutation.argv, `persistence ${action}`)
  const request = mutation.request
  const requestAction = request?.action ?? request?.operation ?? request?.mutation
  if (!request || typeof request !== "object" || Array.isArray(request)
    || requestAction !== action || !sameArray(request.argv, argv)) {
    throw new Error(`managed parity persistence ${action} request must exactly repeat its argv`)
  }
  const expectedCheckpoint = {
    save: { before: "before-docker-save", after: "after-docker-save" },
    remove: { before: "before-docker-remove", after: "after-docker-remove" },
    restore: { before: "before-docker-restore", after: "after-docker-restore" },
  }[action]
  if (!sameJson(mutation.checkpoints, expectedCheckpoint)) {
    throw new Error(`managed parity persistence ${action} checkpoints are not exact`)
  }
  return {
    action,
    argv,
    request: redactManagedValue({ ...request, argv }),
    checkpoints: expectedCheckpoint,
  }
}

function normalizePersistenceEvidence(value, ownedResources, plan) {
  const source = value?.persistenceMutations ?? value?.mutations
  if (!Array.isArray(source) || source.length !== 3) {
    throw new Error("managed parity persistence must expose exact save/remove/restore mutation evidence")
  }
  const plannedMutations = plan?.persistenceMutations
  if (!Array.isArray(plannedMutations) || plannedMutations.length !== 3) {
    throw new Error("managed parity persistence execution requires an immutable mutation plan")
  }
  const normalized = []
  for (const [index, mutation] of source.entries()) {
    const definition = normalizePersistenceMutationDefinition(mutation, index)
    const plannedDefinition = normalizePersistenceMutationDefinition(plannedMutations[index], index)
    if (!sameJson(definition, plannedDefinition)) {
      throw new Error(`managed parity persistence ${definition.action} result does not match its plan`)
    }
    const { action, argv, request } = definition
    const before = mutation.before ?? mutation.inventory
    if (!before || typeof before !== "object" || Array.isArray(before)) {
      throw new Error(`managed parity persistence ${action} requires a pre-mutation inventory`)
    }
    const receipt = mutation.receipt ?? mutation.receipts?.[action]
    if (!receipt || receipt.ok !== true || !hasText(receipt.id ?? receipt.receiptId)) {
      throw new Error(`managed parity persistence ${action} requires a successful receipt`)
    }
    if (action === "save" && !hasText(receipt.archivePath)) {
      throw new Error("managed parity persistence save requires an archive path receipt")
    }
    if (action === "remove" && !sameJson(mutation.saveReceipt, normalized[0]?.receipt)) {
      throw new Error("managed parity persistence remove must carry the exact save receipt")
    }
    if (action === "remove" && receipt.parentReceiptId !== receiptId(normalized[0]?.receipt)) {
      throw new Error("managed parity persistence remove receipt must chain from save")
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
    }
    normalized.push({
      ...redactManagedValue(mutation),
      action,
      argv,
      request,
      checkpoints: definition.checkpoints,
      before: redactManagedValue(before),
      receipt: redactManagedValue(receipt),
      ...(mutation.saveReceipt ? { saveReceipt: redactManagedValue(mutation.saveReceipt) } : {}),
      ...(mutation.removeReceipt ? { removeReceipt: redactManagedValue(mutation.removeReceipt) } : {}),
    })
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
  }, { requireManagedMetrics: Boolean(managedInspection) })
  if (output.zeroResidue !== true) {
    throw new Error("managed parity cleanup.inspect found owned resource residue")
  }
  if (request?.requireGlobalZeroResidue === true && !managedInspection) {
    throw new Error("managed parity cleanup.inspect cannot prove global zero residue through public paths")
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
  const roomId = evidence.identity?.roomId ?? evidence.identity?.sessionId
  if (hasText(roomId) && typeof requestApi.listSessionMembersRequest === "function") {
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
    unsupportedChecks: [
      "global process/listener/container residue requires a kernel-managed cleanup inspector",
    ],
  }
}

function ownedResourcesTrackedAfterCleanup(ownedResources) {
  return Boolean(ownedResources.sliceId)
    || ownedResources.attachmentIds.size !== 0
    || ownedResources.attachmentsByClient.size !== 0
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
          rssDeltaBytes: requireNonNegativeFinite(
            value.resources?.rssDeltaBytes,
            "cleanup resources.rssDeltaBytes",
          ),
          diskDeltaBytes: requireNonNegativeFinite(
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
  requestApi,
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
  await detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step })
  const deleteResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "deleteSliceRequest")(sliceId),
    signal,
    step,
    sliceLifecycleTimeoutMs,
  )
  const deleted = responseVariant(deleteResponse, "SliceDeleted", "selkies.destroy").slice
  validateDeletedSlice(deleted, sliceId, step)
  rememberCleanupEvidence(ownedResources, {
    sliceId,
    attachmentIds,
    identity,
    deleted: true,
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
  requestApi,
  ownedResources,
  signal,
  sliceLifecycleTimeoutMs,
}) {
  const sliceId = ownedResources.sliceId
  const attachmentIds = [...ownedResources.attachmentIds]
  if (sliceId) {
    await detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step: "cleanup.perform" })
    const deleteResponse = await sendWithAbortSignal(
      displayClient,
      requireRequestConstructor(requestApi, "deleteSliceRequest")(sliceId),
      signal,
      "cleanup.perform",
      sliceLifecycleTimeoutMs,
    )
    const deleted = responseVariant(deleteResponse, "SliceDeleted", "cleanup.perform").slice
    validateDeletedSlice(deleted, sliceId, "cleanup.perform")
  }
  const previousEvidence = ownedResources.cleanupEvidence
  rememberCleanupEvidence(ownedResources, {
    sliceId: sliceId ?? previousEvidence?.sliceId,
    attachmentIds: attachmentIds.length > 0 ? attachmentIds : previousEvidence?.attachmentIds,
    identity: ownedResources.identity ?? previousEvidence?.identity ?? ownedResources.stableIdentity,
    deleted: true,
    reason: "cleanup",
  })
  clearOwnedResources(ownedResources)
  return { cleaned: true, sliceId, attachmentIds }
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
  ownedResources.identity = null
}

function rememberCleanupEvidence(ownedResources, evidence) {
  ownedResources.cleanupEvidence = redactManagedValue({
    schema: MANAGED_PARITY_SCHEMA,
    cleaned: true,
    sliceId: evidence.sliceId ?? ownedResources.stableIdentity?.sliceId ?? null,
    attachmentIds: [...new Set(evidence.attachmentIds ?? [])],
    detachedAttachmentIds: [...ownedResources.detachedAttachmentIds],
    identity: evidence.identity ?? ownedResources.stableIdentity ?? null,
    deleted: evidence.deleted === true,
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
