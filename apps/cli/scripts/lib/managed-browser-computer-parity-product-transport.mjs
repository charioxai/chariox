import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"
import {
  createManagedComputerCellRuntimeMcpFromEnvironment,
  inspectManagedComputerCellCleanup,
  runManagedComputerCell,
  runManagedComputerCellReconnect,
  runManagedComputerCellTakeover,
} from "./managed-computer-cell-fixture.mjs"

const OPERATOR_ENDPOINT_ENV = "CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL"
const TARGET_KERNEL_ENV = "CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF"
const TARGET_MACHINE_ENV = "CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF"
const OPERATOR_CLIENT_ENV = "CHARIOX_MANAGED_PARITY_CLIENT_ID"
const OPERATOR_SESSION_ENV = "CHARIOX_MANAGED_PARITY_SESSION_ID"
const DISPLAY_CONNECT_TIMEOUT_MS = 10_000
const DISPLAY_READY_TIMEOUT_MS = 10_000

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
export async function createManagedBrowserComputerParityTransport({ evidenceRoot, signal } = {}) {
  if (typeof evidenceRoot !== "string" || evidenceRoot.trim() === "") {
    throw new Error("managed parity transport requires an external evidenceRoot")
  }

  const homeKernelUrl = requiredOperatorValue(OPERATOR_ENDPOINT_ENV)
  const targetKernelRef = requiredOperatorValue(TARGET_KERNEL_ENV)
  const targetMachineRef = requiredOperatorValue(TARGET_MACHINE_ENV)
  const clientId = requiredOperatorValue(OPERATOR_CLIENT_ENV)
  const sessionId = requiredOperatorValue(OPERATOR_SESSION_ENV)
  requireStandardLocalAuthConfiguration()

  const [{ LocalIpcClient }, requestApi, displayApi] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../../../../packages/kernel-client/dist/display-stream.js"),
  ])
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
    const webSocketModule = createRequire(fileURLToPath(new URL(
      "../../../../packages/kernel-client/dist/ipc.js",
      import.meta.url,
    )))("ws")
    const webSocket = webSocketModule.WebSocket ?? webSocketModule.default ?? webSocketModule
    return {
      ...createManagedBrowserComputerParityTransportFromPublicClient({
        client: workerClient,
        displayClient: homeClient,
        identityClient: workerClient,
        requestApi,
        targetKernelRef,
        targetMachineRef,
        runtimeMcp: createManagedComputerCellRuntimeMcpFromEnvironment(),
        reconnectClient: async () => new LocalIpcClient(homeKernelUrl),
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
  runtimeMcp = null,
  reconnectClient = null,
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
    computer: null,
    cleanupEvidence: null,
  }

  return {
    async run(step, request, { signal } = {}) {
      if (signal?.aborted) {
        throw new Error(`managed parity ${step} was aborted before the public request`)
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
      if (step === "selkies.destroy") {
        return runSelkiesDestroy({
          displayClient,
          requestApi,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "selkies.computer") {
        return runManagedComputerCell({
          displayClient,
          requestApi,
          runtimeMcp,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "selkies.takeover") {
        return runManagedComputerCellTakeover({
          displayClient,
          requestApi,
          runtimeMcp,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "selkies.reconnect") {
        return runManagedComputerCellReconnect({
          displayClient,
          requestApi,
          reconnectClient,
          runtimeMcp,
          ownedResources,
          request,
          signal,
        })
      }
      if (step === "cleanup.perform") {
        return runCleanup({ displayClient, requestApi, ownedResources, signal })
      }
      if (step === "cleanup.inspect") {
        return inspectManagedComputerCellCleanup({
          displayClient,
          requestApi,
          ownedResources,
          request,
          signal,
        })
      }
      throw new Error(`unsupported managed parity step: ${step}`)
    },
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
}) {
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
  )
  const created = responseVariant(createResponse, "SliceCreated", "selkies.create")?.slice
  const createdSliceId = requireText(created?.id, "SliceCreated.slice.id")
  // Record the returned ownership identity before any deeper response
  // validation so cleanup can still delete a resource after a partial failure.
  ownedResources.sliceId = createdSliceId
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
  const displayBackend = observedDisplayBackend(observed, "selkies.create slice observation")

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
  return {
    ...identity,
    displayBackend,
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
}) {
  const sliceId = resolveOwnedSliceId(request, ownedResources, "selkies.destroy", { requireOwned: true })
  const identity = ownedResources.identity
  if (!identity) {
    throw new Error("managed parity selkies.destroy cannot verify the created target identity")
  }
  const attachmentIds = [...ownedResources.attachmentIds]
  await detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step: "selkies.destroy" })
  const deleteResponse = await sendWithAbortSignal(
    displayClient,
    requireRequestConstructor(requestApi, "deleteSliceRequest")(sliceId),
    signal,
    "selkies.destroy",
  )
  const deleted = responseVariant(deleteResponse, "SliceDeleted", "selkies.destroy").slice
  validateDeletedSlice(deleted, sliceId, "selkies.destroy")
  rememberCleanupEvidence(ownedResources, {
    sliceId,
    roomId: identity.roomId,
    attachmentIds,
    detachedAttachmentIds: attachmentIds,
    deleted: true,
    cleaned: true,
  })
  clearOwnedResources(ownedResources)
  return {
    ...identity,
    displayBackend: "selkies",
    destroyed: true,
    sliceId,
    attachmentIds,
  }
}

async function runCleanup({ displayClient, requestApi, ownedResources, signal }) {
  const sliceId = ownedResources.sliceId
  const attachmentIds = [...ownedResources.attachmentIds]
  if (sliceId) {
    await detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step: "cleanup.perform" })
    const deleteResponse = await sendWithAbortSignal(
      displayClient,
      requireRequestConstructor(requestApi, "deleteSliceRequest")(sliceId),
      signal,
      "cleanup.perform",
    )
    const deleted = responseVariant(deleteResponse, "SliceDeleted", "cleanup.perform").slice
    validateDeletedSlice(deleted, sliceId, "cleanup.perform")
  }
  if (sliceId || !ownedResources.cleanupEvidence) {
    rememberCleanupEvidence(ownedResources, {
      sliceId,
      roomId: ownedResources.identity?.roomId ?? ownedResources.computer?.binding?.roomId ?? null,
      attachmentIds,
      detachedAttachmentIds: attachmentIds,
      deleted: Boolean(sliceId),
      cleaned: true,
    })
  }
  clearOwnedResources(ownedResources)
  return { cleaned: true, sliceId, attachmentIds }
}

async function detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step }) {
  const detach = requireRequestConstructor(requestApi, "detachFromSessionRequest")
  for (const attachmentId of ownedResources.attachmentIds) {
    const response = await sendWithAbortSignal(
      displayClient,
      detach(attachmentId),
      signal,
      `${step} attachment detach`,
    )
    const detached = responseVariant(response, "SessionDetached", `${step} attachment detach`).attachment
    validateSessionAttachment(detached, attachmentId, null, `${step} attachment detach`)
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

function observedDisplayBackend(slice, step) {
  const endpoint = slice?.display_endpoint
  if (!endpoint || typeof endpoint !== "object") {
    throw new Error(`managed parity ${step} returned no kernel-selected display endpoint`)
  }
  if (endpoint.slice_id !== slice.id || endpoint.kind !== "selkies") {
    throw new Error(`managed parity ${step} did not return the kernel-selected Selkies backend`)
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
  ownedResources.computer = null
}

function rememberCleanupEvidence(ownedResources, evidence) {
  ownedResources.cleanupEvidence = {
    ...evidence,
    attachmentIds: [...(evidence.attachmentIds ?? [])],
    detachedAttachmentIds: [...(evidence.detachedAttachmentIds ?? [])],
  }
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

async function sendWithAbortSignal(client, request, signal, step) {
  if (signal?.aborted) {
    closeAfterAbort(client)
    throw abortError(step)
  }
  const operation = Promise.resolve().then(() => client.send(request))
  if (!signal) return operation

  let abort
  const aborted = new Promise((_, reject) => {
    abort = () => {
      closeAfterAbort(client)
      reject(abortError(step))
    }
  })
  signal.addEventListener("abort", abort, { once: true })
  if (signal.aborted) abort()
  try {
    return await Promise.race([operation, aborted])
  } finally {
    signal.removeEventListener("abort", abort)
  }
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
