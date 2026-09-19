import { setTimeout as sleep } from "node:timers/promises"

export const MANAGED_COMPUTER_CELL_SCHEMA = "chariox.managed_computer_cell.v1"
export const MANAGED_COMPUTER_CELL_MARKER_PREFIX = "CHARIOX_MANAGED_COMPUTER_CELL"

const DEFAULT_POINTER = Object.freeze({ x: 1, y: 1 })
const DEFAULT_ACTION_WAIT_MS = 20_000
const DEFAULT_POLL_MS = 25

/**
 * Build the public runtime-MCP adapter used by the provider-free Computer
 * cell. The token is an already-authorized runtime token; this helper does
 * not create an agent, start a provider, or add a second authority path.
 */
export function createManagedComputerCellRuntimeMcp({
  serverUrl,
  authToken,
  fetchImpl = globalThis.fetch,
} = {}) {
  requireText(serverUrl, "runtime MCP server URL")
  requireText(authToken, "runtime MCP auth token")
  if (typeof fetchImpl !== "function") {
    throw new Error("managed Computer cell requires the public runtime MCP fetch implementation")
  }

  return {
    async callTool(name, argumentsValue = {}, { signal } = {}) {
      requireText(name, "runtime MCP tool name")
      const response = await fetchImpl(serverUrl, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${authToken}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: `managed-computer-cell-${name}-${Date.now()}`,
          method: "tools/call",
          params: { name, arguments: argumentsValue },
        }),
        signal,
      })
      const bodyText = await response.text()
      let envelope
      try {
        envelope = JSON.parse(bodyText)
      } catch (error) {
        throw new Error(
          `managed Computer cell runtime MCP ${name} returned non-JSON HTTP ${response.status}`,
          { cause: error },
        )
      }
      if (!response.ok || envelope.error) {
        return {
          ok: false,
          error: envelope.error ?? {
            code: `http_${response.status}`,
            message: `runtime MCP returned HTTP ${response.status}`,
          },
          raw: envelope,
        }
      }
      const result = envelope.result
      return {
        ok: result?.isError !== true,
        ...(result?.isError === true ? {
          error: { message: "runtime MCP tool returned isError" },
        } : {}),
        content: result?.structuredContent ?? null,
        raw: envelope,
      }
    },
  }
}

export function createManagedComputerCellRuntimeMcpFromEnvironment(env = process.env) {
  const serverUrl = env.CHARIOX_MANAGED_PARITY_RUNTIME_MCP_URL?.trim()
  const authToken = env.CHARIOX_MANAGED_PARITY_RUNTIME_MCP_TOKEN?.trim()
  if (!serverUrl && !authToken) return null
  if (!serverUrl || !authToken) {
    throw new Error(
      "managed Computer cell requires CHARIOX_MANAGED_PARITY_RUNTIME_MCP_URL and CHARIOX_MANAGED_PARITY_RUNTIME_MCP_TOKEN together",
    )
  }
  return createManagedComputerCellRuntimeMcp({ serverUrl, authToken })
}

export async function runManagedComputerCell({
  displayClient,
  requestApi,
  runtimeMcp,
  ownedResources,
  request,
  signal,
} = {}) {
  requireClient(displayClient, "managed Computer cell Room client")
  requireRuntimeMcp(runtimeMcp)
  const binding = requireBinding(request, "selkies.computer")
  if (request?.displayBackend !== undefined && request.displayBackend !== "selkies") {
    throw new Error("managed Computer cell requires the selkies display backend")
  }
  const roomId = binding.roomId
  const sliceId = requireText(ownedResources?.sliceId, "selkies.computer owned slice")
  const marker = computerMarker(request?.marker, request?.runId)
  const before = await readRoomSnapshot({
    client: displayClient,
    requestApi,
    roomId,
    signal,
    step: "selkies.computer baseline",
  })
  assertEnvironmentBinding(before, binding, "selkies.computer baseline")
  if (ownedResources?.identity) {
    for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
      if (ownedResources.identity[field] !== binding[field]) {
        throw new Error(`selkies.computer binding does not match the owned target identity for ${field}`)
      }
    }
  }
  if (before.lifecycle !== "ready") {
    throw new Error(`managed Computer cell requires a ready Room environment, got ${before.lifecycle}`)
  }

  const keyboard = await runAgentAction({
    displayClient,
    requestApi,
    runtimeMcp,
    requestApiContext: { roomId, sliceId, signal },
    before,
    tool: "slice_keyboard",
    argumentsValue: { action: "type", text: marker },
    expectedKind: "keyboard_text",
    step: "selkies.computer keyboard",
  })
  const pointer = await runAgentAction({
    displayClient,
    requestApi,
    runtimeMcp,
    requestApiContext: { roomId, sliceId, signal },
    before: keyboard.snapshot,
    tool: "slice_mouse",
    argumentsValue: { action: "move", ...pointerFor(request?.pointer) },
    expectedKind: "pointer_move",
    step: "selkies.computer pointer",
  })

  const screenshot = await captureAndVerifyMarker({
    runtimeMcp,
    marker,
    roomId,
    sliceId,
    signal,
    step: "selkies.computer fixture",
  })
  const after = pointer.snapshot
  const events = await readActionEvents({
    client: displayClient,
    requestApi,
    roomId,
    cursor: before.event_cursor,
    signal,
    step: "selkies.computer Room events",
    requiredActionIds: [keyboard.action.action_id, pointer.action.action_id],
  })
  const history = await readActionHistory({
    client: displayClient,
    requestApi,
    roomId,
    signal,
    step: "selkies.computer action history",
    requiredActionIds: [keyboard.action.action_id, pointer.action.action_id],
  })

  ownedResources.computer = {
    schema: MANAGED_COMPUTER_CELL_SCHEMA,
    binding: { ...binding },
    sliceId,
    marker,
    baselineEventCursor: before.event_cursor,
    runtimeGeneration: after.runtime_generation,
    viewportRevision: after.viewport.revision,
    keyboard,
    pointer,
    screenshot,
    events,
    history,
  }
  return {
    ...binding,
    displayBackend: "selkies",
    screenshot: true,
    pointer: true,
    keyboard: true,
    fixture: {
      kind: "inert_desktop_keyboard_marker",
      marker,
      occurrenceCount: screenshot.occurrenceCount,
      artifactId: screenshot.artifactId,
      sha256: screenshot.sha256,
    },
    actorId: keyboard.action.actor_id,
    actorLabel: requireText(
      findActor(after, keyboard.action.actor_id)?.display_label,
      "selkies.computer agent actor label",
    ),
    agentId: keyboard.agentId,
    actionId: keyboard.action.action_id,
    sequence: keyboard.action.sequence,
    runtimeGeneration: keyboard.action.runtime_generation,
    lifecycle: after.lifecycle,
    completion: keyboard.action.state,
    actions: {
      keyboard: summarizeAction(keyboard.action),
      pointer: summarizeAction(pointer.action),
    },
    roomEvents: events.events,
    actionHistory: history.actions.map(summarizeAction),
    roomEventIds: events.eventIds,
    actionHistoryIds: history.actionIds,
  }
}

export async function runManagedComputerCellTakeover({
  displayClient,
  requestApi,
  runtimeMcp,
  ownedResources,
  request,
  signal,
} = {}) {
  requireClient(displayClient, "managed Computer cell takeover Room client")
  requireRuntimeMcp(runtimeMcp)
  const computer = requireComputerState(ownedResources, "selkies.takeover")
  const roomId = computer.binding.roomId
  const target = { kind: "desktop" }
  const takeoverRequest = requireConstructor(
    requestApi,
    "requestRoomEnvironmentInputTakeoverRequest",
  )(roomId, target)
  const takeoverResponse = await sendPublic(
    displayClient,
    takeoverRequest,
    signal,
    "selkies.takeover request",
  )
  const takeover = responseVariant(
    takeoverResponse,
    "RoomEnvironmentTakeoverUpdated",
    "selkies.takeover request",
  )
  if (takeover.outcome?.state !== "granted") {
    throw new Error(
      `managed Computer cell takeover did not grant desktop ownership: ${JSON.stringify(takeover.outcome)}`,
    )
  }
  const humanOwnership = findOwnership(takeover.environment, target)
  const humanActorId = requireText(humanOwnership?.actor_id, "selkies.takeover human actor ID")
  const humanActor = findActor(takeover.environment, humanActorId)
  if (humanActor?.kind !== "human") {
    throw new Error("managed Computer cell takeover returned a non-human desktop owner")
  }
  if (request?.humanActorId !== undefined && request.humanActorId !== humanActorId) {
    throw new Error("managed Computer cell takeover returned an unexpected human actor")
  }

  const idempotency = await replayIdempotentHumanAction({
    displayClient,
    requestApi,
    roomId,
    environment: takeover.environment,
    request,
    signal,
  })
  const actionIdsBeforeRejectedAgent = new Set(
    idempotency.after.snapshot.actions.map((action) => action.action_id),
  )
  const rejected = await callRuntimeTool(
    runtimeMcp,
    "slice_keyboard",
    {
      action: "type",
      text: `${computer.marker}_TAKEOVER_REJECTED`,
    },
    signal,
    "selkies.takeover agent mutation rejection",
  )
  assertRejectedRuntimeReceipt(rejected, "selkies.takeover agent mutation rejection")
  const afterRejectedAgent = await readRoomSnapshot({
    client: displayClient,
    requestApi,
    roomId,
    signal,
    step: "selkies.takeover rejected agent snapshot",
  })
  const newActionIds = afterRejectedAgent.actions
    .map((action) => action.action_id)
    .filter((actionId) => !actionIdsBeforeRejectedAgent.has(actionId))
  if (newActionIds.length !== 0) {
    throw new Error("managed Computer cell rejected agent mutation but the Room added an action")
  }

  const releaseResponse = await sendPublic(
    displayClient,
    sendReleaseRequest(requestApi, roomId, target),
    signal,
    "selkies.takeover release",
  )
  const released = responseVariant(
    releaseResponse,
    "RoomEnvironmentInputReleased",
    "selkies.takeover release",
  ).environment
  if (findOwnership(released, target) || findPendingTakeover(released, target)) {
    throw new Error("managed Computer cell release left desktop ownership or a pending takeover")
  }

  computer.takeover = {
    target,
    humanActorId,
    humanActorLabel: requireText(humanActor?.display_label, "selkies.takeover human actor label"),
    rejected,
    idempotency,
    released,
  }
  return {
    ...computer.binding,
    displayBackend: "selkies",
    overlayVisible: true,
    takeoverCompleted: true,
    actorAttributed: true,
    humanActorId,
    humanActorLabel: humanActor.display_label,
    agentMutationRejected: true,
    agentMutationRejection: summarizeRejection(rejected),
    idempotency: {
      key: idempotency.key,
      actionId: idempotency.actionId,
      replayActionId: idempotency.replayActionId,
      sequence: idempotency.sequence,
      duplicateHistoryEntries: idempotency.duplicateHistoryEntries,
    },
    released: true,
  }
}

export async function runManagedComputerCellReconnect({
  displayClient,
  requestApi,
  reconnectClient,
  runtimeMcp,
  ownedResources,
  request,
  signal,
} = {}) {
  requireRuntimeMcp(runtimeMcp)
  const computer = requireComputerState(ownedResources, "selkies.reconnect")
  if (typeof reconnectClient !== "function") {
    throw new Error("managed Computer cell selkies.reconnect requires a public reconnect client factory")
  }
  const client = await reconnectClient()
  requireClient(client, "managed Computer cell reconnect client")
  try {
    const snapshot = await readRoomSnapshot({
      client,
      requestApi,
      roomId: computer.binding.roomId,
      signal,
      step: "selkies.reconnect snapshot",
    })
    assertEnvironmentBinding(snapshot, computer.binding, "selkies.reconnect snapshot")
    if (snapshot.lifecycle !== "ready") {
      throw new Error(`managed Computer cell reconnect returned lifecycle ${snapshot.lifecycle}`)
    }
    const expectedActionIds = [
      computer.keyboard.action.action_id,
      computer.pointer.action.action_id,
      ...(computer.takeover?.idempotency?.actionId ? [computer.takeover.idempotency.actionId] : []),
    ]
    const history = await readActionHistory({
      client,
      requestApi,
      roomId: computer.binding.roomId,
      signal,
      step: "selkies.reconnect action history",
      requiredActionIds: expectedActionIds,
    })
    const events = await readActionEvents({
      client,
      requestApi,
      roomId: computer.binding.roomId,
      cursor: computer.baselineEventCursor,
      signal,
      step: "selkies.reconnect Room events",
      requiredActionIds: expectedActionIds,
    })
    const actionIdCounts = new Map()
    for (const action of history.actions) {
      actionIdCounts.set(action.action_id, (actionIdCounts.get(action.action_id) ?? 0) + 1)
    }
    const duplicateActions = [...actionIdCounts.values()]
      .reduce((count, occurrences) => count + Math.max(0, occurrences - 1), 0)
    if (duplicateActions !== 0) {
      throw new Error("managed Computer cell reconnect found duplicate action-history entries")
    }
    const inventory = await readReconnectInventory({
      client,
      requestApi,
      roomId: computer.binding.roomId,
      sliceId: computer.sliceId,
      signal,
    })
    const screenshot = await captureAndVerifyMarker({
      runtimeMcp,
      marker: computer.marker,
      roomId: computer.binding.roomId,
      sliceId: computer.sliceId,
      signal,
      step: "selkies.reconnect fixture",
    })
    const stale = request?.staleProbe === true
      ? await probeStalePointer({
        client,
        requestApi,
        roomId: computer.binding.roomId,
        snapshot,
        request,
        signal,
      })
      : { status: "not_requested" }
    computer.reconnect = {
      snapshot,
      history,
      events,
      inventory,
      screenshot,
      stale,
    }
    return {
      ...computer.binding,
      displayBackend: "selkies",
      faultInjected: request?.fault === "relay_disconnect",
      reconnected: true,
      duplicateActions,
      duplicateBrowsers: inventory.browserCount - 1,
      marker: computer.marker,
      screenshot: true,
      fixtureArtifactId: screenshot.artifactId,
      roomEvents: events.events,
      actionHistory: history.actions.map(summarizeAction),
      eventCount: events.eventIds.length,
      historyCount: history.actions.length,
      stale,
      runtimeGeneration: snapshot.runtime_generation,
      lifecycle: snapshot.lifecycle,
    }
  } finally {
    if (typeof client.close === "function") await client.close().catch(() => {})
  }
}

export async function inspectManagedComputerCellCleanup({
  displayClient,
  requestApi,
  ownedResources,
  request,
  signal,
} = {}) {
  requireClient(displayClient, "managed Computer cell cleanup Room client")
  const evidence = ownedResources?.cleanupEvidence
  if (!evidence?.cleaned) {
    throw new Error("managed Computer cell cleanup.inspect requires a cleanup.perform receipt")
  }
  if (request?.requireGlobalZeroResidue === true) {
    throw new Error(
      "managed Computer cell cleanup.inspect cannot prove global process/listener/container residue through the public kernel client",
    )
  }
  const listSlices = requireConstructor(requestApi, "listSlicesRequest")
  const listSessions = requireConstructor(requestApi, "listSessionsRequest")
  const slicesResponse = await sendPublic(
    displayClient,
    listSlices(),
    signal,
    "cleanup.inspect slices",
  )
  const slices = requireArray(
    responseVariant(slicesResponse, "SlicesListed", "cleanup.inspect slices").slices,
    "cleanup.inspect SlicesListed.slices",
  )
  const sessionsResponse = await sendPublic(
    displayClient,
    listSessions(),
    signal,
    "cleanup.inspect sessions",
  )
  const sessions = requireArray(
    responseVariant(sessionsResponse, "SessionsListed", "cleanup.inspect sessions").sessions,
    "cleanup.inspect SessionsListed.sessions",
  )
  if (slices.some((slice) => slice?.id === evidence.sliceId)) {
    throw new Error("managed Computer cell cleanup.inspect found the owned slice after cleanup")
  }
  if (ownedResources.sliceId || ownedResources.attachmentIds?.size !== 0) {
    throw new Error("managed Computer cell cleanup.inspect found tracked owned resources")
  }
  if (evidence.attachmentIds.length !== evidence.detachedAttachmentIds.length) {
    throw new Error("managed Computer cell cleanup.inspect lacks a detach receipt for every attachment")
  }
  return {
    schema: MANAGED_COMPUTER_CELL_SCHEMA,
    inspected: true,
    zeroResidue: true,
    owned: {
      sliceId: evidence.sliceId,
      attachmentIds: [...evidence.attachmentIds],
      detachedAttachmentIds: [...evidence.detachedAttachmentIds],
      deleted: evidence.deleted,
    },
    publicInventory: {
      ownedSliceCount: slices.filter((slice) => slice?.id === evidence.sliceId).length,
      roomPresent: sessions.some((session) => session?.id === evidence.roomId),
      receiptKinds: ["SlicesListed", "SessionsListed", "SliceDeleted", "SessionDetached"],
    },
    unsupportedChecks: [
      "managedMachines",
      "processes",
      "listeners",
      "containers",
      "temporaryFiles",
      "retainedEvidenceLeakCount",
    ],
  }
}

async function replayIdempotentHumanAction({
  displayClient,
  requestApi,
  roomId,
  environment,
  request,
  signal,
}) {
  const submit = requireConstructor(requestApi, "submitRoomEnvironmentActionRequest")
  const key = requireText(
    request?.idempotencyKey ?? `${environment.session_id}-managed-computer-idempotency`,
    "selkies.takeover idempotency key",
  )
  const pointer = pointerFor(request?.idempotencyPointer)
  const submitRequest = submit(
    roomId,
    requireSafeInteger(environment.runtime_generation, "selkies.takeover runtime generation"),
    requireSafeInteger(environment.viewport?.revision, "selkies.takeover viewport revision"),
    key,
    { kind: "pointer_move", ...pointer },
  )
  const firstResponse = await sendPublic(
    displayClient,
    submitRequest,
    signal,
    "selkies.takeover idempotent action",
  )
  const first = responseVariant(
    firstResponse,
    "RoomEnvironmentActionSubmitted",
    "selkies.takeover idempotent action",
  )
  const firstActionId = requireText(first.action_id, "selkies.takeover action ID")
  const after = await waitForActionState({
    client: displayClient,
    requestApi,
    roomId,
    actionId: firstActionId,
    signal,
    step: "selkies.takeover idempotent action completion",
  })
  if (after.action.idempotency_key !== key || after.action.actor_id !== findOwnership(after.snapshot, { kind: "desktop" })?.actor_id) {
    throw new Error("managed Computer cell idempotent action lost its Room actor or idempotency identity")
  }
  const replayResponse = await sendPublic(
    displayClient,
    submitRequest,
    signal,
    "selkies.takeover idempotent replay",
  )
  const replay = responseVariant(
    replayResponse,
    "RoomEnvironmentActionSubmitted",
    "selkies.takeover idempotent replay",
  )
  if (replay.action_id !== firstActionId) {
    throw new Error("managed Computer cell idempotent replay allocated a second action")
  }
  const history = await readActionHistory({
    client: displayClient,
    requestApi,
    roomId,
    signal,
    step: "selkies.takeover idempotent history",
    requiredActionIds: [firstActionId],
  })
  const duplicateHistoryEntries = history.actions
    .filter((action) => action.action_id === firstActionId).length - 1
  if (duplicateHistoryEntries !== 0) {
    throw new Error("managed Computer cell idempotent replay duplicated the Room action history")
  }
  return {
    key,
    actionId: firstActionId,
    replayActionId: replay.action_id,
    sequence: after.action.sequence,
    action: after.action,
    after,
    duplicateHistoryEntries,
  }
}

async function runAgentAction({
  displayClient,
  requestApi,
  runtimeMcp,
  requestApiContext,
  before,
  tool,
  argumentsValue,
  expectedKind,
  step,
}) {
  const receipt = await callRuntimeTool(runtimeMcp, tool, argumentsValue, requestApiContext.signal, step)
  if (receipt?.ok !== true) {
    throw new Error(`${step} did not return a successful public runtime receipt`)
  }
  const content = runtimeContent(receipt, step)
  if (content.source !== "computer_controller") {
    throw new Error(`${step} returned a non-Computer runtime source`)
  }
  if (content.session_id !== requestApiContext.roomId || content.slice_id !== requestApiContext.sliceId) {
    throw new Error(`${step} returned a receipt for the wrong Room or slice`)
  }
  const agentId = requireText(content.agent_id, `${step} agent ID`)
  const actionId = requireText(content.action_id, `${step} action ID`)
  const actorId = requireText(content.actor_id, `${step} actor ID`)
  if (content.action_kind !== expectedKind) {
    throw new Error(`${step} returned action kind ${content.action_kind ?? "missing"}, expected ${expectedKind}`)
  }
  const runtimeGeneration = requireSafeInteger(content.runtime_generation, `${step} runtime generation`)
  const settled = await waitForActionState({
    client: displayClient,
    requestApi,
    roomId: requestApiContext.roomId,
    actionId,
    signal: requestApiContext.signal,
    step,
  })
  const snapshot = settled.snapshot
  const action = settled.action
  if (action.actor_id !== actorId || action.runtime_generation !== runtimeGeneration) {
    throw new Error(`${step} Room action did not preserve the runtime actor or generation receipt`)
  }
  if (findActor(snapshot, action.actor_id)?.kind !== "agent") {
    throw new Error(`${step} Room action was not attributed to an agent actor`)
  }
  if (action.mode !== "computer" || action.kind !== expectedKind || action.state !== "completed") {
    throw new Error(`${step} Room action did not complete as the expected Computer action`)
  }
  if (action.outcome?.status !== "completed") {
    throw new Error(`${step} Room action has no completed lifecycle outcome`)
  }
  if (!Array.isArray(action.targets) || !action.targets.some((target) => target?.kind === "desktop")) {
    throw new Error(`${step} Room action has no desktop target receipt`)
  }
  if (action.sequence <= before.actions.reduce((max, candidate) => Math.max(max, candidate.sequence ?? 0), 0)) {
    throw new Error(`${step} Room action sequence is not newer than the baseline`)
  }
  return {
    receipt: summarizeRuntimeReceipt(receipt),
    agentId,
    action,
    snapshot,
  }
}

async function captureAndVerifyMarker({ runtimeMcp, marker, roomId, sliceId, signal, step }) {
  const screenshotReceipt = await callRuntimeTool(
    runtimeMcp,
    "slice_screenshot",
    { return_image_base64: false },
    signal,
    `${step} screenshot`,
  )
  if (screenshotReceipt?.ok !== true || !screenshotReceipt.content) {
    throw new Error(`${step} did not return a successful screenshot receipt`)
  }
  const screenshot = runtimeContent(screenshotReceipt, `${step} screenshot`)
  if (screenshot.source !== "computer_controller"
    || screenshot.session_id !== roomId
    || screenshot.slice_id !== sliceId) {
    throw new Error(`${step} screenshot returned the wrong Room or slice authority`)
  }
  const artifactId = requireText(screenshot.artifact_id, `${step} screenshot artifact ID`)
  const sha256 = requireText(screenshot.sha256, `${step} screenshot SHA-256`)
  const sizeBytes = requireSafeInteger(screenshot.size_bytes, `${step} screenshot size`)
  const ocrReceipt = await callRuntimeTool(
    runtimeMcp,
    "slice_ocr",
    { artifact_id: artifactId },
    signal,
    `${step} OCR`,
  )
  if (ocrReceipt?.ok !== true) throw new Error(`${step} did not return a successful OCR receipt`)
  const ocr = runtimeContent(ocrReceipt, `${step} OCR`)
  if (ocr.source !== "computer_controller"
    || ocr.session_id !== roomId
    || ocr.slice_id !== sliceId) {
    throw new Error(`${step} OCR returned the wrong Room or slice authority`)
  }
  const text = requireText(ocr.text, `${step} OCR text`)
  const occurrenceCount = countOccurrences(text, marker)
  if (occurrenceCount !== 1) {
    throw new Error(`${step} expected the inert marker exactly once in the screenshot OCR`)
  }
  return { artifactId, sha256, sizeBytes, occurrenceCount }
}

async function waitForActionState({ client, requestApi, roomId, actionId, signal, step }) {
  const deadline = Date.now() + DEFAULT_ACTION_WAIT_MS
  while (true) {
    const snapshot = await readRoomSnapshot({ client, requestApi, roomId, signal, step })
    const action = snapshot.actions.find((candidate) => candidate?.action_id === actionId)
    if (action?.state === "completed") return { snapshot, action }
    if (action && ["failed", "cancelled"].includes(action.state)) {
      throw new Error(`${step} action ${actionId} ended in ${action.state}`)
    }
    if (Date.now() >= deadline) {
      throw new Error(`${step} action ${actionId} did not reach a terminal completed receipt`)
    }
    await sleep(DEFAULT_POLL_MS)
  }
}

async function readRoomSnapshot({ client, requestApi, roomId, signal, step }) {
  const response = await sendPublic(
    client,
    requireConstructor(requestApi, "getRoomEnvironmentStateRequest")(roomId),
    signal,
    step,
  )
  const environment = responseVariant(response, "RoomEnvironmentState", step).environment
  if (!environment || typeof environment !== "object") throw new Error(`${step} returned no Room environment snapshot`)
  requireText(environment.session_id, `${step} session ID`)
  requireText(environment.environment_id, `${step} environment ID`)
  if (!Array.isArray(environment.actions) || !Array.isArray(environment.input_ownership)
    || !Array.isArray(environment.pending_input_takeovers)) {
    throw new Error(`${step} omitted authoritative actions or input ownership`)
  }
  if (!environment.viewport || !Number.isSafeInteger(environment.viewport.revision)) {
    throw new Error(`${step} omitted the authoritative viewport revision`)
  }
  return environment
}

async function readActionEvents({
  client,
  requestApi,
  roomId,
  cursor,
  signal,
  step,
  requiredActionIds,
}) {
  const response = await sendPublic(
    client,
    requireConstructor(requestApi, "getRoomEnvironmentEventsRequest")(roomId, cursor),
    signal,
    step,
  )
  const replay = responseVariant(response, "RoomEnvironmentEvents", step).replay
  if (!replay?.Events || !Array.isArray(replay.Events.events)) {
    throw new Error(`${step} did not return an event replay receipt`)
  }
  const events = replay.Events.events
  const eventIds = events.map((event) => requireSafeInteger(event?.event_id, `${step} event ID`))
  for (const actionId of requiredActionIds) {
    if (!events.some((event) => event?.kind?.ActionChanged?.action_id === actionId)) {
      throw new Error(`${step} omitted ActionChanged for ${actionId}`)
    }
  }
  return {
    eventIds,
    events: events.map((event) => ({
      eventId: event.event_id,
      actionId: event.kind?.ActionChanged?.action_id ?? null,
      state: event.kind?.ActionChanged?.state ?? null,
    })),
    nextCursor: requireSafeInteger(replay.Events.next_cursor, `${step} next cursor`),
  }
}

async function readActionHistory({
  client,
  requestApi,
  roomId,
  signal,
  step,
  requiredActionIds,
}) {
  const response = await sendPublic(
    client,
    requireConstructor(requestApi, "listRoomEnvironmentActionHistoryRequest")(roomId, null, 200),
    signal,
    step,
  )
  const page = responseVariant(response, "RoomEnvironmentActionHistoryListed", step).page
  if (!page || !Array.isArray(page.actions)) throw new Error(`${step} omitted the action history page`)
  const actionIds = page.actions.map((action) => requireText(action?.action_id, `${step} action ID`))
  for (const actionId of requiredActionIds) {
    if (page.actions.filter((action) => action.action_id === actionId).length !== 1) {
      throw new Error(`${step} did not retain exactly one action-history entry for ${actionId}`)
    }
  }
  return { actions: page.actions, actionIds }
}

async function readReconnectInventory({ client, requestApi, roomId, sliceId, signal }) {
  const response = await sendPublic(
    client,
    requireConstructor(requestApi, "getRoomEnvironmentResourceInventoryRequest")(roomId, sliceId),
    signal,
    "selkies.reconnect resource inventory",
  )
  const inventory = responseVariant(
    response,
    "RoomEnvironmentResourceInventory",
    "selkies.reconnect resource inventory",
  ).inventory
  if (!inventory || inventory.session_id !== roomId || inventory.slice_id !== sliceId) {
    throw new Error("managed Computer cell reconnect returned the wrong resource inventory identity")
  }
  const browserIds = uniqueStrings(inventory.browser_ids, "selkies.reconnect browser IDs")
  const profileIds = uniqueStrings(inventory.profile_ids, "selkies.reconnect profile IDs")
  if (browserIds.length !== 1 || profileIds.length !== 1) {
    throw new Error("managed Computer cell reconnect requires exactly one browser and profile receipt")
  }
  return { browserCount: browserIds.length, profileCount: profileIds.length }
}

async function probeStalePointer({ client, requestApi, roomId, snapshot, request, signal }) {
  const staleGeneration = request.staleRuntimeGeneration
  if (!Number.isSafeInteger(staleGeneration) || staleGeneration >= snapshot.runtime_generation) {
    throw new Error("managed Computer cell stale probe requires an older runtime generation")
  }
  const pointerRequest = requireConstructor(requestApi, "updateRoomEnvironmentPointerRequest")(
    roomId,
    staleGeneration,
    snapshot.viewport.revision,
    null,
  )
  try {
    const response = await sendPublic(client, pointerRequest, signal, "selkies.reconnect stale probe")
    throw new Error(`managed Computer cell stale probe was accepted: ${JSON.stringify(response)}`)
  } catch (error) {
    if (error instanceof Error && /stale probe was accepted/.test(error.message)) throw error
    if (error?.name === "AbortError"
      || !/stale|generation|viewport|outdated|invalid/i.test(String(error?.message ?? error))) {
      throw error
    }
    return { status: "rejected", runtimeGeneration: staleGeneration }
  }
}

function sendReleaseRequest(requestApi, roomId, target) {
  return requireConstructor(requestApi, "releaseRoomEnvironmentInputRequest")(roomId, target)
}

async function callRuntimeTool(runtimeMcp, name, argumentsValue, signal, step) {
  if (typeof runtimeMcp?.callTool !== "function") {
    throw new Error(`${step} requires the public runtime MCP tool receipt`)
  }
  const result = await runtimeMcp.callTool(name, argumentsValue, { signal })
  if (!result || typeof result !== "object") throw new Error(`${step} returned no runtime MCP receipt`)
  return result
}

function assertRejectedRuntimeReceipt(receipt, step) {
  if (receipt.ok === true || (receipt.isError !== true && !receipt.error)) {
    throw new Error(`${step} did not return an explicit public rejection receipt`)
  }
  const errorText = JSON.stringify(receipt.error ?? receipt.raw ?? receipt)
  if (!/takeover|ownership|permission|human|input/i.test(errorText)) {
    throw new Error(`${step} returned an unrelated or unstructured rejection receipt`)
  }
}

function runtimeContent(receipt, step) {
  const content = receipt.content ?? receipt.structuredContent ?? receipt.payload
  if (!content || typeof content !== "object") throw new Error(`${step} omitted structured runtime content`)
  return content
}

function summarizeRuntimeReceipt(receipt) {
  const content = runtimeContent(receipt, "runtime receipt")
  return {
    source: content.source,
    sessionId: content.session_id,
    sliceId: content.slice_id,
    agentId: content.agent_id,
    actorId: content.actor_id,
    actionId: content.action_id,
    actionKind: content.action_kind,
    runtimeGeneration: content.runtime_generation,
  }
}

function summarizeAction(action) {
  return {
    actionId: action.action_id,
    sequence: action.sequence,
    actorId: action.actor_id,
    runtimeGeneration: action.runtime_generation,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
    outcome: action.outcome,
  }
}

function summarizeRejection(receipt) {
  return { error: receipt.error ?? receipt.raw ?? null }
}

function requireComputerState(ownedResources, step) {
  if (!ownedResources?.computer) throw new Error(`managed Computer cell ${step} requires selkies.computer first`)
  return ownedResources.computer
}

function requireRuntimeMcp(runtimeMcp) {
  if (typeof runtimeMcp?.callTool !== "function") {
    throw new Error("managed Computer cell requires a public runtime MCP client; provider execution is not accepted")
  }
}

function requireClient(client, label) {
  if (!client || typeof client.send !== "function") throw new Error(`${label} is required`)
}

function requireBinding(request, step) {
  const binding = request?.binding
  if (!binding || typeof binding !== "object") throw new Error(`managed parity ${step} requires a target binding`)
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) requireText(binding[field], `${step} binding.${field}`)
  return binding
}

function assertEnvironmentBinding(environment, binding, step) {
  if (environment.session_id !== binding.roomId || environment.environment_id !== binding.environmentId) {
    throw new Error(`${step} returned a different Room or environment identity`)
  }
}

function findOwnership(environment, target) {
  return environment?.input_ownership?.find((owner) => sameTarget(owner?.target, target))
}

function findPendingTakeover(environment, target) {
  return environment?.pending_input_takeovers?.find((pending) => sameTarget(pending?.target, target))
}

function findActor(environment, actorId) {
  return environment?.actors?.find((actor) => actor?.actor_id === actorId)
}

function sameTarget(left, right) {
  return left?.kind === right?.kind && (left?.kind !== "browser_tab" || left?.id === right?.id)
}

function pointerFor(value) {
  const pointer = value ?? DEFAULT_POINTER
  if (!Number.isSafeInteger(pointer.x) || !Number.isSafeInteger(pointer.y)
    || pointer.x < 0 || pointer.y < 0) {
    throw new Error("managed Computer cell pointer coordinates must be non-negative integers")
  }
  return { x: pointer.x, y: pointer.y }
}

function computerMarker(value, runId) {
  const candidate = value ?? `${MANAGED_COMPUTER_CELL_MARKER_PREFIX}_${safeToken(runId ?? "run")}`
  const marker = requireText(candidate, "managed Computer cell marker")
  if (!/^[A-Za-z0-9_-]{8,128}$/.test(marker)) {
    throw new Error("managed Computer cell marker must be a bounded inert ASCII token")
  }
  return marker
}

function safeToken(value) {
  return String(value).replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 80)
}

function countOccurrences(text, needle) {
  let count = 0
  let offset = 0
  while (true) {
    const index = text.indexOf(needle, offset)
    if (index === -1) return count
    count += 1
    offset = index + needle.length
  }
}

function requireConstructor(requestApi, name) {
  if (typeof requestApi?.[name] !== "function") {
    throw new Error(`managed Computer cell requires released kernel request constructor ${name}`)
  }
  return requestApi[name]
}

function responseVariant(response, variant, step) {
  if (!response || typeof response !== "object" || !(variant in response)) {
    throw new Error(`${step} returned an unexpected response; expected ${variant}`)
  }
  return response[variant]
}

async function sendPublic(client, request, signal, step) {
  if (signal?.aborted) throw abortError(step)
  const operation = Promise.resolve().then(() => client.send(request))
  if (!signal) return operation
  let abort
  const aborted = new Promise((_, reject) => {
    abort = () => reject(abortError(step))
    signal.addEventListener("abort", abort, { once: true })
  })
  try {
    return await Promise.race([operation, aborted])
  } finally {
    signal.removeEventListener("abort", abort)
  }
}

function abortError(step) {
  const error = new Error(`managed Computer cell ${step} was aborted while the public request was in flight`)
  error.name = "AbortError"
  return error
}

function requireText(value, label) {
  if (typeof value !== "string" || value.trim() === "") throw new Error(`managed Computer cell requires ${label}`)
  return value
}

function requireSafeInteger(value, label) {
  if (!Number.isSafeInteger(value)) throw new Error(`managed Computer cell requires ${label}`)
  return value
}

function requireArray(value, label) {
  if (!Array.isArray(value)) throw new Error(`managed Computer cell requires ${label}`)
  return value
}

function uniqueStrings(value, label) {
  const values = requireArray(value, label).map((item) => requireText(item, `${label} identity`))
  if (new Set(values).size !== values.length) throw new Error(`managed Computer cell requires unique ${label}`)
  return values
}
