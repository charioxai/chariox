import {
  DrillChaosClock,
  DrillChaosProcess,
  DrillChaosRandom,
  DrillChaosTrace,
  DrillChaosTransport,
  createDrillChaosReplayBundle,
} from "./drill-deterministic-chaos.mjs"
import {
  assertDrillRuntimeConvergenceInvariants,
  evaluateDrillRuntimeConvergenceInvariants,
} from "./drill-runtime-convergence-invariants.mjs"

export const DETERMINISTIC_RUNTIME_CHAOS_SCENARIO = "deterministic-runtime-convergence"
export const DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED = "arroba-runtime-chaos-v1"

export const DETERMINISTIC_RUNTIME_CHAOS_FAULT_PLAN = Object.freeze([
  fault({
    id: "request-drop-twice",
    kind: "drop",
    match: { channel: "request", operationId: "op-retry" },
    times: 2,
  }),
  fault({
    id: "request-duplicate",
    kind: "duplicate",
    match: { channel: "request", operationId: "op-duplicate" },
    copies: 3,
  }),
  fault({
    id: "request-delay",
    kind: "delay",
    match: { channel: "request", operationId: "op-delay" },
    delayMs: 5,
  }),
  fault({
    id: "event-reorder",
    kind: "reorder",
    match: { channel: "event", to: "tui" },
    times: 2,
    window: 2,
  }),
  fault({
    id: "web-event-partition",
    kind: "route-partition",
    match: { from: "kernel", to: "web" },
  }),
  fault({
    id: "web-snapshot-reconnect",
    kind: "route-reconnect",
    match: { from: "kernel", to: "web" },
  }),
  fault({
    id: "provider-process-death",
    kind: "process-death",
    match: { from: "provider", to: "kernel" },
  }),
  fault({
    id: "provider-stale-callback",
    kind: "stale-callback",
    match: { from: "provider", to: "kernel" },
  }),
])

export async function createDeterministicRuntimeChaosReplay({
  seed = DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED,
} = {}) {
  const random = new DrillChaosRandom(seed)
  const clock = new DrillChaosClock()
  const trace = new DrillChaosTrace(() => clock.nowMs())
  clock.trace = trace

  const kernelState = state()
  const clients = {
    tui: clientState(),
    web: clientState(),
  }
  const acceptedOperations = new Set()
  const executionCounts = new Map()
  const authorityMutations = []
  let staleCallbacksExecuted = 0

  let eventTransport
  const requestTransport = new DrillChaosTransport({
    name: "client-to-kernel",
    clock,
    trace,
    maxQueueDepth: 32,
    faultPlan: DETERMINISTIC_RUNTIME_CHAOS_FAULT_PLAN.filter((item) => (
      item.kind === "drop" || item.kind === "delay" || item.kind === "duplicate"
    )),
    deliver: async (packet) => {
      const operation = packet.payload
      const executionCount = executionCounts.get(operation.id) ?? 0
      if (executionCount > 0) {
        trace.record("kernel.duplicate-operation-ignored", { operationId: operation.id })
        return
      }
      executionCounts.set(operation.id, executionCount + 1)
      kernelState.cursor += 1
      kernelState.operationIds.push(operation.id)
      kernelState.values[operation.key] = operation.value
      authorityMutations.push({ owner: "kernel-authority", operationId: operation.id })
      trace.record("kernel.operation-executed", { operationId: operation.id, cursor: kernelState.cursor })
      for (const clientId of Object.keys(clients)) {
        eventTransport.send({
          messageId: `event-${kernelState.cursor}-${clientId}`,
          operationId: operation.id,
          channel: "event",
          from: "kernel",
          to: clientId,
          type: "runtime-event",
          payload: {
            cursor: kernelState.cursor,
            operation,
          },
        })
      }
    },
  })
  eventTransport = new DrillChaosTransport({
    name: "kernel-to-client",
    clock,
    trace,
    maxQueueDepth: 32,
    faultPlan: DETERMINISTIC_RUNTIME_CHAOS_FAULT_PLAN.filter((item) => item.kind === "reorder"),
    deliver: async (packet) => applyClientPacket(clients[packet.to], packet, trace),
  })

  trace.record("chaos.fault-applied", { faultId: "web-event-partition", faultKind: "route-partition" })
  eventTransport.partition({ from: "kernel", to: "web", mode: "hold" })
  const providerProcess = new DrillChaosProcess({ name: "provider", clock, trace })
  trace.record("chaos.fault-applied", { faultId: "provider-stale-callback", faultKind: "stale-callback" })
  providerProcess.schedule("late-provider-output", 3, async () => {
    staleCallbacksExecuted += 1
    authorityMutations.push({ owner: "provider", operationId: "stale-provider-output" })
  })
  clock.setTimeout(() => {
    trace.record("chaos.fault-applied", { faultId: "provider-process-death", faultKind: "process-death" })
    providerProcess.kill("deterministic process-death fault")
  }, 1, { faultId: "provider-process-death" })
  clock.setTimeout(() => providerProcess.restart(), 2, { faultId: "provider-process-restart" })

  const operations = random.shuffle([
    operation("op-retry", "prompt", "queued-and-retried"),
    operation("op-duplicate", "permission", "approved-once"),
    operation("op-delay", "workflow", "completed-after-delay"),
  ])
  for (const item of operations) {
    acceptedOperations.add(item.id)
    executionCounts.set(item.id, 0)
    trace.record("client.operation-accepted", { operationId: item.id, source: item.source })
    sendOperation(requestTransport, item, 1)
    if (item.id === "op-retry") {
      clock.setTimeout(() => sendOperation(requestTransport, item, 2), 2, { operationId: item.id, attempt: 2 })
      clock.setTimeout(() => sendOperation(requestTransport, item, 3), 4, { operationId: item.id, attempt: 3 })
    }
  }

  clock.setTimeout(() => {
    trace.record("chaos.fault-applied", { faultId: "web-snapshot-reconnect", faultKind: "route-reconnect" })
    eventTransport.reconnect({ from: "kernel", to: "web", release: "drop" })
    eventTransport.send({
      messageId: `snapshot-${kernelState.cursor}-web`,
      channel: "snapshot",
      from: "kernel",
      to: "web",
      type: "runtime-snapshot",
      payload: structuredClone(kernelState),
    })
  }, 8, { faultId: "web-snapshot-reconnect" })

  await clock.runUntilIdle()
  eventTransport.flushReorderBuffers()
  await clock.runUntilIdle()

  const resources = {
    clockTasks: clock.pendingCount(),
    eventTransportPackets: eventTransport.pendingCount(),
    providerCallbacks: providerProcess.pendingCount(),
    requestTransportPackets: requestTransport.pendingCount(),
    tuiBufferedEvents: clients.tui.pendingEvents.size,
    webBufferedEvents: clients.web.pendingEvents.size,
  }
  const invariants = assertDrillRuntimeConvergenceInvariants(evaluateDrillRuntimeConvergenceInvariants({
    acceptedOperations,
    executionCounts,
    kernelState,
    clientStates: {
      tui: clients.tui,
      web: clients.web,
    },
    cursorHistory: {
      tui: clients.tui.cursorHistory,
      web: clients.web.cursorHistory,
    },
    authorityMutations,
    queue: {
      limit: 64,
      maxObserved: requestTransport.maxObservedQueueDepth + eventTransport.maxObservedQueueDepth,
      pending: requestTransport.pendingCount() + eventTransport.pendingCount(),
    },
    resources,
    staleCallbacks: {
      scheduled: 1,
      suppressed: providerProcess.staleCallbacksSuppressed,
      executed: staleCallbacksExecuted,
    },
  }))

  return createDrillChaosReplayBundle({
    scenario: DETERMINISTIC_RUNTIME_CHAOS_SCENARIO,
    random,
    faultPlan: DETERMINISTIC_RUNTIME_CHAOS_FAULT_PLAN,
    trace,
    invariants,
    metadata: {
      clients: Object.keys(clients),
      operationOrder: operations.map((item) => item.id),
      virtualDurationMs: clock.nowMs(),
    },
  })
}

function applyClientPacket(client, packet, trace) {
  if (packet.type === "runtime-snapshot") {
    client.cursor = packet.payload.cursor
    client.operationIds = [...packet.payload.operationIds]
    client.values = { ...packet.payload.values }
    client.pendingEvents.clear()
    client.cursorHistory.push(client.cursor)
    trace.record("client.snapshot-applied", { clientId: packet.to, cursor: client.cursor })
    return
  }
  const { cursor, operation: item } = packet.payload
  if (cursor <= client.cursor) {
    trace.record("client.duplicate-event-ignored", { clientId: packet.to, cursor })
    return
  }
  client.pendingEvents.set(cursor, item)
  while (client.pendingEvents.has(client.cursor + 1)) {
    const nextCursor = client.cursor + 1
    const next = client.pendingEvents.get(nextCursor)
    client.pendingEvents.delete(nextCursor)
    client.cursor = nextCursor
    if (!client.operationIds.includes(next.id)) client.operationIds.push(next.id)
    client.values[next.key] = next.value
    client.cursorHistory.push(client.cursor)
    trace.record("client.event-applied", { clientId: packet.to, operationId: next.id, cursor: client.cursor })
  }
}

function sendOperation(transport, item, attempt) {
  transport.send({
    messageId: `request-${item.id}-${attempt}`,
    operationId: item.id,
    channel: "request",
    from: item.source,
    to: "kernel",
    type: "runtime-operation",
    payload: item,
  })
}

function state() {
  return { cursor: 0, operationIds: [], values: {} }
}

function clientState() {
  return { ...state(), cursorHistory: [0], pendingEvents: new Map() }
}

function operation(id, key, value) {
  return { id, key, value, source: id === "op-duplicate" ? "web" : "tui" }
}

function fault(value) {
  return Object.freeze(value)
}
