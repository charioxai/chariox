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
const RESOURCE_TELEMETRY_TIMEOUT_MS = 10_000
const PERSISTENCE_TIMEOUT_MS = 30_000
const CLEANUP_INSPECTION_TIMEOUT_MS = 15_000
const MANAGED_PARITY_SCHEMA = "chariox.browser_computer_m0_guard.v1"

/**
 * Load the released public client modules at runtime. Keeping this seam
 * injectable lets source-only tests exercise the transport without creating
 * ignored dist output or weakening the production import boundary.
 */
export async function loadManagedBrowserComputerParityKernelClientModules() {
  const [{ LocalIpcClient }, requestApi, displayApi] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../../../../packages/kernel-client/dist/display-stream.js"),
  ])
  return { LocalIpcClient, requestApi, displayApi }
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
    ?? (firstCallable(identityClient, ["describePersistenceMutations", "runPersistenceMutations"]) ? identityClient : null)
  const residueAdapter = cleanupInspector
    ?? residueInspector
    ?? (firstCallable(identityClient, ["inspectCleanupResidue", "cleanupInspect"]) ? identityClient : null)
  const timeoutConfig = {
    resourceTelemetryMs: finiteTimeout(timeouts.resourceTelemetryMs, RESOURCE_TELEMETRY_TIMEOUT_MS),
    persistenceMs: finiteTimeout(timeouts.persistenceMs, PERSISTENCE_TIMEOUT_MS),
    cleanupInspectionMs: finiteTimeout(timeouts.cleanupInspectionMs, CLEANUP_INSPECTION_TIMEOUT_MS),
  }

  const transport = {
    resourceScope: "managed-target",
    targetId: targetMachineRef ?? targetKernelRef ?? null,
    targetKernelRef,
    targetMachineRef,
    async collectManagedTargetResourceSnapshot({ phase, sampleId, evidenceRoot, now, signal } = {}) {
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
        })
      }
      if (step === "cleanup.perform") {
        return runCleanup({ displayClient, requestApi, ownedResources, signal })
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
  now,
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
  const capturedAt = normalizeTimestamp(now)
  const normalized = redactManagedValue({
    ...candidate,
    schema: candidate.schema ?? MANAGED_PARITY_SCHEMA,
    phase: phase ?? candidate.phase ?? "unspecified",
    sampleId: sampleId ?? candidate.sampleId ?? null,
    capturedAt: candidate.capturedAt ?? capturedAt,
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
    "getDaemonHealthRequest",
  ].find((name) => typeof requestApi?.[name] === "function")
  if (!requestName) {
    throw new Error("managed parity remote transport has no kernel-managed resource telemetry path")
  }
  const request = requestName === "getDaemonHealthRequest"
    ? requestApi[requestName]()
    : requestApi[requestName]({
      kernelRef: input.targetKernelRef,
      machineRef: input.targetMachineRef,
    })
  const response = await sendWithAbortSignal(client, request, signal, "resource telemetry")
  const telemetry = unwrapManagedTelemetry(response)
  if (requestName !== "getDaemonHealthRequest") return telemetry
  if (!telemetry || typeof telemetry !== "object" || Array.isArray(telemetry)) return telemetry
  return {
    ...telemetry,
    process: telemetry.process
      ? {
        ...telemetry.process,
        rssBytes: telemetry.process.rssBytes
          ?? telemetry.process.current_resident_set_bytes
          ?? null,
      }
      : telemetry.process,
    telemetry: telemetry.telemetry ?? {
      scope: "managed-target",
      authoritative: true,
      targetId: input.targetMachineRef ?? input.targetKernelRef ?? null,
      source: "kernel-daemon-health",
    },
  }
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
  return normalizePersistenceEvidence(raw, ownedResources)
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
  return normalizePersistenceEvidence(raw, ownedResources)
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

function normalizePersistenceEvidence(value, ownedResources) {
  const source = value?.persistenceMutations ?? value?.mutations
  if (!Array.isArray(source) || source.length !== 3) {
    throw new Error("managed parity persistence must expose exact save/remove/restore mutation evidence")
  }
  const actions = ["save", "remove", "restore"]
  const normalized = []
  for (const [index, mutation] of source.entries()) {
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
      request: redactManagedValue({ ...request, argv }),
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
          ...(publicInspection.unsupportedChecks ?? []),
          ...(managedInspection.unsupportedChecks ?? []),
        ],
      }
      : {}),
    owned: stableOwnedIdentity(ownedResources),
  })
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

function normalizeCleanupInspection(value) {
  const output = redactManagedValue({
    schema: value?.schema ?? MANAGED_PARITY_SCHEMA,
    inspected: value?.inspected === true,
    zeroResidue: value?.zeroResidue === true,
    owned: value?.owned ?? null,
    publicInventory: value?.publicInventory ?? null,
    ownedSliceCount: numericOrZero(value?.ownedSliceCount),
    ownedAttachmentResidueCount: numericOrZero(value?.ownedAttachmentResidueCount),
    sessionCount: numericOrZero(value?.sessionCount),
    memberInspection: value?.memberInspection ?? "unknown",
    unsupportedChecks: Array.isArray(value?.unsupportedChecks) ? value.unsupportedChecks : [],
    resources: {
      rssDeltaBytes: numericOrZero(value?.resources?.rssDeltaBytes),
      diskDeltaBytes: numericOrZero(value?.resources?.diskDeltaBytes),
    },
  })
  for (const field of [
    "managedMachines", "rooms", "environments", "processes", "listeners", "containers", "profiles",
    "activeTargets", "temporaryFiles", "retainedEvidenceLeakCount",
  ]) {
    if (value?.[field] !== undefined) output[field] = numericOrZero(value[field])
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

function numericOrZero(value) {
  return Number.isFinite(value) && value >= 0 ? value : 0
}

function finiteTimeout(value, fallback) {
  return Number.isFinite(value) && value > 0 ? Math.min(value, 120_000) : fallback
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
  ownedResources.stableIdentity = {
    ...identity,
    sliceId: ownedResources.sliceId,
  }
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
    attachmentIds,
    identity,
    deleted: true,
    reason: "destroy",
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
  rememberCleanupEvidence(ownedResources, {
    sliceId,
    attachmentIds,
    identity: ownedResources.identity ?? ownedResources.stableIdentity,
    deleted: true,
    reason: "cleanup",
  })
  clearOwnedResources(ownedResources)
  return { cleaned: true, sliceId, attachmentIds }
}

async function detachOwnedAttachments({ displayClient, requestApi, ownedResources, signal, step }) {
  if (ownedResources.attachmentIds.size === 0) return
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
  return withDeadline(
    () => client.send(request),
    {
      signal,
      timeoutMs: PUBLIC_REQUEST_TIMEOUT_MS,
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
