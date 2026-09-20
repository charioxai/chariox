import assert from "node:assert/strict"
import test from "node:test"

import {
  createManagedBrowserComputerParityTransport,
  createManagedBrowserComputerParityTransportFromPublicClient,
} from "./managed-browser-computer-parity-product-transport.mjs"

const moduleRequestApi = {
  getSliceDisplayEndpointRequest(sliceId, options) {
    return { GetSliceDisplayEndpoint: { slice_ref: sliceId, ...options } }
  },
}

test("factory uses the released kernel telemetry request without importing dist", async () => {
  const previous = new Map([
    ["CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL", process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL],
    ["CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF", process.env.CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF],
    ["CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF", process.env.CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF],
    ["CHARIOX_MANAGED_PARITY_CLIENT_ID", process.env.CHARIOX_MANAGED_PARITY_CLIENT_ID],
    ["CHARIOX_MANAGED_PARITY_SESSION_ID", process.env.CHARIOX_MANAGED_PARITY_SESSION_ID],
    ["CHARIOX_KERNEL_LOCAL_AUTH_TOKEN", process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN],
  ])
  Object.assign(process.env, {
    CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL: "ws://home.invalid",
    CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF: "kernel-1",
    CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF: "machine-1",
    CHARIOX_MANAGED_PARITY_CLIENT_ID: "client-1",
    CHARIOX_MANAGED_PARITY_SESSION_ID: "session-1",
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: "operator-token",
  })
  class SourceSeamClient {
    constructor(url) {
      this.url = url
    }

    async send(request) {
      if (this.url === "ws://home.invalid") {
        assert.ok(Object.hasOwn(request, "ResolveKernelClientConnection"))
        return {
          KernelClientConnectionResolved: {
            connection: {
              relay_url: "ws://worker.invalid",
              relay_token: "relay-token-kept-in-memory",
              target_daemon_id: "daemon-1",
              target_daemon_alias: null,
            },
          },
        }
      }
      assert.deepEqual(request, {
        GetKernelResourceTelemetry: null,
      })
      return { KernelResourceTelemetry: { snapshot: managedTelemetry() } }
    }

    async close() {}
  }
  try {
    const transport = await createManagedBrowserComputerParityTransport({
      evidenceRoot: "/tmp/managed-parity-source-seam",
      kernelClientModules: {
        LocalIpcClient: SourceSeamClient,
        requestApi: {
          ...moduleRequestApi,
          resolveKernelClientConnectionRequest(options) {
            return { ResolveKernelClientConnection: options }
          },
          relayStatusRequest() { return { RelayStatus: null } },
          getRoomEnvironmentStateRequest(sessionId) {
            return { GetRoomEnvironmentState: { session_id: sessionId } }
          },
          getKernelResourceTelemetryRequest() {
            return { GetKernelResourceTelemetry: null }
          },
        },
        displayApi: { openSelkiesDisplayStream() {} },
        webSocket: {},
      },
    })
    assert.equal(transport.targetId, "machine-1")
    const sample = await transport.collectManagedTargetResourceSnapshot({ phase: "preflight" })
    assert.equal(sample.telemetry.source, "kernel-managed-test")
    assert.equal(sample.telemetry.targetId, "machine-1")
    await transport.close()
  } finally {
    for (const [key, value] of previous) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
})

test("factory fails before returning a transport when only daemon health is available", async () => {
  const previous = new Map([
    ["CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL", process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL],
    ["CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF", process.env.CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF],
    ["CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF", process.env.CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF],
    ["CHARIOX_MANAGED_PARITY_CLIENT_ID", process.env.CHARIOX_MANAGED_PARITY_CLIENT_ID],
    ["CHARIOX_MANAGED_PARITY_SESSION_ID", process.env.CHARIOX_MANAGED_PARITY_SESSION_ID],
    ["CHARIOX_KERNEL_LOCAL_AUTH_TOKEN", process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN],
  ])
  Object.assign(process.env, {
    CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL: "ws://home.invalid",
    CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF: "kernel-1",
    CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF: "machine-1",
    CHARIOX_MANAGED_PARITY_CLIENT_ID: "client-1",
    CHARIOX_MANAGED_PARITY_SESSION_ID: "session-1",
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: "operator-token",
  })
  class DaemonHealthOnlyClient {
    async send(request) {
      assert.ok(Object.hasOwn(request, "ResolveKernelClientConnection"))
      return {
        KernelClientConnectionResolved: {
          connection: {
            relay_url: "ws://worker.invalid",
            relay_token: "relay-token-kept-in-memory",
            target_daemon_id: "daemon-1",
            target_daemon_alias: null,
          },
        },
      }
    }

    async close() {}
  }
  try {
    await assert.rejects(
      () => createManagedBrowserComputerParityTransport({
        evidenceRoot: "/tmp/managed-parity-source-seam",
        kernelClientModules: {
          LocalIpcClient: DaemonHealthOnlyClient,
          requestApi: {
            ...moduleRequestApi,
            resolveKernelClientConnectionRequest(options) {
              return { ResolveKernelClientConnection: options }
            },
            getDaemonHealthRequest() { return { GetDaemonHealth: null } },
            relayStatusRequest() { return { RelayStatus: null } },
            getRoomEnvironmentStateRequest(sessionId) {
              return { GetRoomEnvironmentState: { session_id: sessionId } }
            },
          },
          displayApi: { openSelkiesDisplayStream() {} },
          webSocket: {},
        },
      }),
      /requires a complete kernel-managed resource telemetry path/,
    )
  } finally {
    for (const [key, value] of previous) {
      if (value === undefined) delete process.env[key]
      else process.env[key] = value
    }
  }
})

function createTelemetryTransport(sample, options = {}) {
  return createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send() { throw new Error("unexpected public telemetry request") } },
    requestApi: moduleRequestApi,
    targetKernelRef: "kernel-1",
    targetMachineRef: "machine-1",
    resourceTelemetry: async () => sample,
    timeouts: options.timeouts,
  })
}

function managedTelemetry(overrides = {}) {
  return {
    telemetry: {
      scope: "managed-target",
      authoritative: true,
      targetId: "machine-1",
      source: "kernel-managed-test",
    },
    capturedAt: "2026-09-20T00:00:00.000Z",
    memory: { totalBytes: 8_000, usedBytes: 4_000, availableBytes: 4_000 },
    disk: { totalBytes: 16_000, usedBytes: 4_000, availableBytes: 12_000 },
    process: { count: 1, rssBytes: 100 },
    logs: { bytes: 0 },
    docker: { containers: [] },
    ...overrides,
  }
}

test("managed resource telemetry passes with stable target identity and redaction", async () => {
  const transport = createTelemetryTransport({
    ...managedTelemetry(),
    providerAuth: "provider-secret-must-not-escape",
  })
  const result = await transport.collectManagedTargetResourceSnapshot({
    phase: "before-browser-start",
    sampleId: "run-1:before-browser-start",
    now: () => new Date("2026-09-20T00:00:00.000Z"),
  })

  assert.equal(result.telemetry.scope, "managed-target")
  assert.equal(result.telemetry.authoritative, true)
  assert.equal(result.telemetry.targetId, "machine-1")
  assert.equal(result.providerAuth, "[REDACTED]")
  assert.equal(result.phase, "before-browser-start")
  assert.equal(result.capturedAt, "2026-09-20T00:00:00.000Z")
})

test("managed resource telemetry does not promote partial daemon health into an authoritative sample", async () => {
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: {
      async send() {
        throw new Error("daemon health must not be requested as resource telemetry")
      },
    },
    requestApi: {
      ...moduleRequestApi,
      getDaemonHealthRequest() { return { GetDaemonHealth: null } },
    },
    targetKernelRef: "kernel-1",
    targetMachineRef: "machine-1",
  })
  await assert.rejects(
    () => transport.collectManagedTargetResourceSnapshot({ phase: "health" }),
    /no kernel-managed resource telemetry path/,
  )
})

test("managed resource telemetry fails closed for a foreign target", async () => {
  const transport = createTelemetryTransport(managedTelemetry({
    telemetry: { ...managedTelemetry().telemetry, targetId: "machine-foreign" },
  }))
  await assert.rejects(
    () => transport.collectManagedTargetResourceSnapshot({ phase: "during" }),
    /foreign target identity/,
  )
})

test("managed resource telemetry reports unknown data instead of using a host fallback", async () => {
  const transport = createTelemetryTransport({ status: "unknown" })
  await assert.rejects(
    () => transport.collectManagedTargetResourceSnapshot({ phase: "after" }),
    /resource telemetry is unknown/,
  )
})

test("managed resource telemetry rejects authoritative metadata without the complete guard sample", async () => {
  const transport = createTelemetryTransport({
    telemetry: managedTelemetry().telemetry,
    process: { rssBytes: 128 },
  })
  await assert.rejects(
    () => transport.collectManagedTargetResourceSnapshot({ phase: "during" }),
    /resource telemetry memory\.totalBytes must be a finite non-negative number/,
  )
})

test("managed resource telemetry rejects missing used bytes and kernel capture time", async () => {
  const withoutMemoryUsed = managedTelemetry({
    memory: { totalBytes: 8_000, availableBytes: 4_000 },
  })
  await assert.rejects(
    () => createTelemetryTransport(withoutMemoryUsed).collectManagedTargetResourceSnapshot({ phase: "during" }),
    /resource telemetry memory\.usedBytes must be a finite non-negative number/,
  )

  const withoutCaptureTime = managedTelemetry({ capturedAt: null })
  await assert.rejects(
    () => createTelemetryTransport(withoutCaptureTime).collectManagedTargetResourceSnapshot({ phase: "during" }),
    /capturedAt must be a kernel-captured timestamp/,
  )
})

test("managed resource telemetry has a finite timeout", async () => {
  const transport = createTelemetryTransport(new Promise(() => {}), {
    timeouts: { resourceTelemetryMs: 5 },
  })
  await assert.rejects(
    () => transport.collectManagedTargetResourceSnapshot({ phase: "during" }),
    /resource telemetry timed out after 5ms/,
  )
})

function createCompatibilityTransport(protocol) {
  const requestApi = {
    ...moduleRequestApi,
    kernelResourceTelemetryMinimumProtocolVersion: protocol,
    getKernelResourceTelemetryRequest() { return { GetKernelResourceTelemetry: null } },
    relayStatusRequest() { return { RelayStatus: null } },
    getRoomEnvironmentStateRequest(sessionId) {
      return { GetRoomEnvironmentState: { session_id: sessionId } }
    },
  }
  const client = {
    async send(request) {
      if (Object.hasOwn(request, "RelayStatus")) {
        return { RelayStatus: { status: {
          configured: true,
          connected: true,
          daemon_id: "kernel-1",
          machine_id: "machine-1",
        } } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentState")) {
        return { RoomEnvironmentState: { environment: {
          session_id: request.GetRoomEnvironmentState.session_id,
          environment_id: "environment-1",
        } } }
      }
      throw new Error(`unexpected compatibility request: ${JSON.stringify(request)}`)
    },
  }
  return createManagedBrowserComputerParityTransportFromPublicClient({
    client,
    requestApi,
    targetKernelRef: "kernel-1",
    targetMachineRef: "machine-1",
    protocolApi: { LOCAL_DAEMON_PROTOCOL_VERSION: 334 },
    parityConfig: { expected: { roomId: "room-1", environmentId: "environment-1" } },
  })
}

test("compatibility preflight rejects stale and too-new kernel protocol constants before target inspection", async () => {
  for (const protocol of [322, 335]) {
    const transport = createCompatibilityTransport(protocol)
    await assert.rejects(
      () => transport.assertCompatibilityPreflight(),
      /requires released kernel protocol 334|observed 322|observed 335/,
      `protocol ${protocol}`,
    )
  }
})

test("transport dispatches every live M0 operation through an explicit operation boundary", async () => {
  const steps = [
    "preflight",
    "selkies.create", "selkies.attach", "selkies.providers", "selkies.browser", "selkies.computer",
    "selkies.takeover", "selkies.persistence", "selkies.vault", "selkies.git", "selkies.reconnect", "selkies.destroy",
    "novnc.create", "novnc.attach", "novnc.rollback", "novnc.destroy",
    "cleanup.perform", "cleanup.inspect",
  ]
  const seen = []
  const operationAdapter = Object.fromEntries(steps.map((step) => [step, async ({ request }) => {
    seen.push(step)
    return { step, ...(request?.binding ?? {}) }
  }]))
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send() { throw new Error("operation adapter must own every step") } },
    requestApi: moduleRequestApi,
    operationAdapter,
  })
  for (const step of steps) await transport.run(step, { binding: { roomId: "room-1" } })
  assert.deepEqual(seen, steps)
})

function persistenceEvidence() {
  const inventory = {
    containers: ["chariox-slice-owned"],
    images: ["chariox/browser:fixture"],
    volumes: [],
    networks: [],
  }
  const saveArgv = ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/browser-state.tar"]
  const removeArgv = ["docker", "rm", "chariox-slice-owned"]
  const restoreArgv = ["docker", "load", "-i", "/tmp/browser-state.tar"]
  const saveReceipt = { ok: true, id: "save-receipt-1", archivePath: "/tmp/browser-state.tar" }
  const removeReceipt = { ok: true, id: "remove-receipt-1", parentReceiptId: saveReceipt.id }
  return {
    persistenceMutations: [
      {
        action: "save",
        argv: saveArgv,
        request: { action: "save", argv: saveArgv },
        before: inventory,
        receipt: saveReceipt,
        checkpoints: { before: "before-docker-save", after: "after-docker-save" },
      },
      {
        action: "remove",
        argv: removeArgv,
        request: { action: "remove", argv: removeArgv },
        before: inventory,
        saveReceipt,
        receipt: removeReceipt,
        checkpoints: { before: "before-docker-remove", after: "after-docker-remove" },
      },
      {
        action: "restore",
        argv: restoreArgv,
        request: { action: "restore", argv: restoreArgv },
        before: inventory,
        saveReceipt,
        removeReceipt,
        receipt: {
          ok: true,
          id: "restore-receipt-1",
          parentReceiptId: removeReceipt.id,
          archivePath: "/tmp/browser-state.tar",
        },
        checkpoints: { before: "before-docker-restore", after: "after-docker-restore" },
      },
    ],
  }
}

function persistencePlan() {
  return {
    persistenceMutations: persistenceEvidence().persistenceMutations.map(({
      action,
      argv,
      request,
      checkpoints,
    }) => ({ action, argv, request, checkpoints })),
  }
}

test("persistence plans remain immutable and execution returns post-mutation receipts", async () => {
  const evidence = persistenceEvidence()
  const plan = persistencePlan()
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send() { throw new Error("unexpected persistence public request") } },
    requestApi: moduleRequestApi,
    persistence: {
      async describe() { return plan },
      async run({ plan: executionPlan, onPersistenceMutation }) {
        for (const mutation of evidence.persistenceMutations) {
          await onPersistenceMutation?.({ phase: "before", mutation })
          await onPersistenceMutation?.({ phase: "after", mutation })
        }
        assert.deepEqual(executionPlan.persistenceMutations, plan.persistenceMutations)
        return evidence
      },
    },
  })

  const described = await transport.describePersistenceMutations({ runId: "run-1" })
  assert.deepEqual(
    described.persistenceMutations.map(({ action, argv, request }) => ({ action, argv, request })),
    plan.persistenceMutations.map(({ action, argv, request }) => ({ action, argv, request })),
  )
  assert.ok(described.persistenceMutations.every((mutation) => !Object.hasOwn(mutation, "receipt")))

  const events = []
  const result = await transport.run("selkies.persistence", {}, {
    onPersistenceMutation: (event) => events.push([event.phase, event.mutation.action]),
  })
  assert.deepEqual(events, [
    ["before", "save"], ["after", "save"],
    ["before", "remove"], ["after", "remove"],
    ["before", "restore"], ["after", "restore"],
  ])
  assert.deepEqual(result.persistenceMutations.map((mutation) => mutation.argv), [
    ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/browser-state.tar"],
    ["docker", "rm", "chariox-slice-owned"],
    ["docker", "load", "-i", "/tmp/browser-state.tar"],
  ])
})

test("persistence rejects receipts declared before execution and no-op plan results", async () => {
  const predeclared = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send() { throw new Error("unexpected persistence public request") } },
    requestApi: moduleRequestApi,
    persistence: {
      async describe() { return persistenceEvidence() },
      async run() { throw new Error("execution must not start") },
    },
  })
  await assert.rejects(
    () => predeclared.describePersistenceMutations({ runId: "run-predeclared" }),
    /must not contain pre-execution .* evidence/,
  )

  const noOp = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send() { throw new Error("unexpected persistence public request") } },
    requestApi: moduleRequestApi,
    persistence: {
      async describe() { return persistencePlan() },
      async run({ plan, onPersistenceMutation }) {
        for (const mutation of plan.persistenceMutations) {
          await onPersistenceMutation?.({ phase: "before", mutation })
          await onPersistenceMutation?.({ phase: "after", mutation })
        }
        return plan
      },
    },
  })
  await assert.rejects(
    () => noOp.run("selkies.persistence", {}),
    /requires a pre-mutation inventory/,
  )
})

test("production persistence defaults to authoritative slice save, stop, and restore requests", async () => {
  const { transport, client } = createCleanupTransport()
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await transport.run("selkies.create", {
    runId: "run-production-persistence",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  const plan = await transport.describePersistenceMutations({ runId: "run-production-persistence" })
  assert.deepEqual(plan.persistenceMutations.map(({ action }) => action), ["save", "remove", "restore"])
  const result = await transport.run("selkies.persistence", { binding })
  assert.equal(result.saved, true)
  assert.equal(result.restarted, true)
  assert.equal(result.sameRoom, true)
  assert.equal(result.sameEnvironment, true)
  assert.equal(result.sameProfile, true)
  assert.deepEqual(result.persistenceMutations.map(({ action }) => action), ["save", "remove", "restore"])
  assert.deepEqual(client.requests.filter((request) => [
    "SaveSliceState", "StopSlice", "StartSlice", "GetSliceStateStatus",
  ].some((variant) => Object.hasOwn(request, variant))).map((request) => Object.keys(request)[0]).slice(-4), [
    "SaveSliceState", "StopSlice", "StartSlice", "GetSliceStateStatus",
  ])
})

function createCleanupTransport({
  deleteRemovesSlice = true,
  startDelayMs = 0,
  detachAckLossAttachmentId = null,
  residualBrowserId = null,
  cleanupInspector = null,
  timeouts,
} = {}) {
  let slicePresent = true
  let attachmentSequence = 0
  let ackLossInjected = false
  const activeAttachmentIds = new Set()
  const detachAttempts = []
  const requests = []
  const slice = {
    id: "slice-1",
    backend: "ssh_docker",
    display_mode: "headed",
    worker_kernel_ref: "worker-ref-1",
    worker_kernel_id: "kernel-1",
    worker_machine_id: "machine-1",
    environment_session_id: "room-1",
    session_id: "room-1",
    status: "running",
    display_endpoint: { slice_id: "slice-1", kind: "selkies" },
  }
  let savedState = null
  const requestApi = {
    ...moduleRequestApi,
    attachToSessionRequest(sessionId, clientId) {
      return { AttachToSession: { session_id: sessionId, client_id: clientId } }
    },
    createSliceRequest(options) { return { CreateSlice: options } },
    bindRoomEnvironmentSliceRequest(sessionId, sliceId) {
      return { BindRoomEnvironmentSlice: { session_id: sessionId, slice_ref: sliceId } }
    },
    startSliceRequest(sliceId) { return { StartSlice: { slice_ref: sliceId } } },
    stopSliceRequest(sliceId) { return { StopSlice: { slice_ref: sliceId } } },
    saveSliceStateRequest(sliceId, mode, scope) {
      return { SaveSliceState: { slice_ref: sliceId, mode, scope } }
    },
    getSliceStateStatusRequest(sliceId) { return { GetSliceStateStatus: { slice_ref: sliceId } } },
    getSliceRequest(sliceId) { return { GetSlice: { slice_ref: sliceId } } },
    listSessionsRequest() { return { ListSessions: null } },
    listSlicesRequest() { return { ListSlices: null } },
    getRoomEnvironmentResourceInventoryRequest(sessionId, sliceId) {
      return { GetRoomEnvironmentResourceInventory: { session_id: sessionId, slice_id: sliceId } }
    },
    getRoomEnvironmentStateRequest(sessionId) {
      return { GetRoomEnvironmentState: { session_id: sessionId } }
    },
    relayStatusRequest() { return { RelayStatus: null } },
    deleteSliceRequest(sliceId) { return { DeleteSlice: { slice_ref: sliceId } } },
    detachFromSessionRequest(attachmentId) {
      return { DetachFromSession: { attachment_id: attachmentId } }
    },
  }
  const client = {
    requests,
    async send(request) {
      requests.push(request)
      if (Object.hasOwn(request, "CreateSlice")) {
        return { SliceCreated: { slice: { ...slice, status: "stopped", worker_kernel_id: null, worker_machine_id: null, environment_session_id: null, session_id: null } } }
      }
      if (Object.hasOwn(request, "BindRoomEnvironmentSlice")) {
        return { RoomEnvironmentSlice: { binding: { session_id: "room-1", slice_id: "slice-1", owner_kernel_id: "kernel-1", worker_kernel_ref: "worker-ref-1" } } }
      }
      if (Object.hasOwn(request, "StartSlice") || Object.hasOwn(request, "GetSlice")) {
        if (Object.hasOwn(request, "StartSlice") && startDelayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, startDelayMs))
        }
        if (Object.hasOwn(request, "StartSlice")) slice.status = "running"
        return Object.hasOwn(request, "StartSlice")
          ? { SliceStarted: { slice } }
          : { Slice: { slice } }
      }
      if (Object.hasOwn(request, "SaveSliceState")) {
        savedState = {
          id: "saved-state-1",
          source_slice_id: "slice-1",
          home_archive_path: "/tmp/managed-parity-slice-state.tar",
        }
        return { SliceStateSaved: { slice, state: savedState } }
      }
      if (Object.hasOwn(request, "StopSlice")) {
        slice.status = "stopped"
        return { SliceStopped: { slice } }
      }
      if (Object.hasOwn(request, "GetSliceStateStatus")) {
        return { SliceStateStatus: { slice, state: savedState } }
      }
      if (Object.hasOwn(request, "RelayStatus")) {
        return { RelayStatus: { status: { configured: true, connected: true, daemon_id: "kernel-1", machine_id: "machine-1" } } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentState")) {
        return { RoomEnvironmentState: { environment: {
          session_id: "room-1",
          environment_id: "environment-1",
          runtime_generation: 1,
          lifecycle: slicePresent ? "ready" : "stopped",
          health: ["browser_controller", "browser", "desktop", "streamer"].map((component) => ({
            component,
            state: slicePresent ? "ready" : "unavailable",
            diagnostic_code: null,
          })),
          viewport: { revision: 1 },
          tabs: [],
          actions: [],
          input_ownership: [],
          pending_input_takeovers: [],
        } } }
      }
      if (Object.hasOwn(request, "ListSessions")) {
        return { SessionsListed: { sessions: slicePresent ? [{ id: "room-1", attachment_ids: [...activeAttachmentIds] }] : [] } }
      }
      if (Object.hasOwn(request, "ListSlices")) {
        return { SlicesListed: { slices: slicePresent ? [slice] : [] } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentResourceInventory")) {
        return { RoomEnvironmentResourceInventory: { inventory: {
          session_id: "room-1",
          environment_id: "environment-1",
          slice_id: "slice-1",
          browser_ids: slicePresent ? ["browser-1"] : (residualBrowserId ? [residualBrowserId] : []),
          profile_ids: slicePresent ? ["profile-1"] : (residualBrowserId ? ["profile-1"] : []),
        } } }
      }
      if (Object.hasOwn(request, "AttachToSession")) {
        const attachmentId = `attachment-${++attachmentSequence}`
        activeAttachmentIds.add(attachmentId)
        return { SessionAttached: { attachment: { id: attachmentId, session_id: "room-1" } } }
      }
      if (Object.hasOwn(request, "DetachFromSession")) {
        const attachmentId = request.DetachFromSession.attachment_id
        detachAttempts.push(attachmentId)
        if (!activeAttachmentIds.delete(attachmentId)) {
          throw new Error(`AttachmentNotFound: ${attachmentId}`)
        }
        if (attachmentId === detachAckLossAttachmentId && !ackLossInjected) {
          ackLossInjected = true
          throw new Error(`simulated detach acknowledgement loss: ${attachmentId}`)
        }
        return { SessionDetached: { attachment: { id: attachmentId, session_id: "room-1" } } }
      }
      if (Object.hasOwn(request, "DeleteSlice")) {
        if (deleteRemovesSlice) slicePresent = false
        return { SliceDeleted: { slice: { id: "slice-1" } } }
      }
      throw new Error(`unexpected cleanup request: ${JSON.stringify(request)}`)
    },
  }
  return {
    transport: createManagedBrowserComputerParityTransportFromPublicClient({
      client,
      requestApi,
      targetKernelRef: "worker-ref-1",
      targetMachineRef: "machine-1",
      cleanupInspector,
      timeouts,
      displayTransport: {
        async openSelkiesDisplayStream() {
          let receiveCount = 0
          return {
            endpoint: {
              stream_protocol: "chariox-display-v1",
              stream_id: "stream-1",
            },
            async sendControl() {},
            async receive() {
              if (receiveCount++ === 0) {
                return { kind: "text", data: Uint8Array.from(Buffer.from("VIDEO_STARTED")) }
              }
              return { kind: "binary", data: Uint8Array.from([4, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]) }
            },
            async close() {},
          }
        },
        webSocket: {},
      },
    }),
    client,
    detachAttempts,
    requests,
  }
}

test("cleanup.inspect proves the stable owned slice has no public residue", async () => {
  const { transport } = createCleanupTransport()
  await transport.run("selkies.create", {
    runId: "run-1",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding: {
      kernelId: "kernel-1",
      machineId: "machine-1",
      roomId: "room-1",
      environmentId: "environment-1",
    },
  })
  await transport.run("cleanup.perform", {})
  const inspection = await transport.run("cleanup.inspect", {})

  assert.equal(inspection.zeroResidue, true)
  assert.equal(inspection.owned.sliceId, "slice-1")
  assert.equal(inspection.owned.deleted, true)
  assert.equal(inspection.ownedSliceCount, 0)
  assert.equal(inspection.ownedAttachmentResidueCount, 0)
  assert.deepEqual(inspection.unsupportedChecks, [])
})

test("cleanup.inspect queries authoritative browser resources and fails closed on residue", async () => {
  const { transport } = createCleanupTransport({ residualBrowserId: "browser-left-behind" })
  await transport.run("selkies.create", {
    runId: "run-residue",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding: {
      kernelId: "kernel-1",
      machineId: "machine-1",
      roomId: "room-1",
      environmentId: "environment-1",
    },
  })
  await transport.run("cleanup.perform", {})
  await assert.rejects(
    () => transport.run("cleanup.inspect", {}),
    /residual browser\/controller\/viewer resources/,
  )
})

test("cleanup.inspect fails when the owned slice remains after a delete receipt", async () => {
  const { transport } = createCleanupTransport({ deleteRemovesSlice: false })
  await transport.run("selkies.create", {
    runId: "run-1",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding: {
      kernelId: "kernel-1",
      machineId: "machine-1",
      roomId: "room-1",
      environmentId: "environment-1",
    },
  })
  await transport.run("cleanup.perform", {})
  await assert.rejects(
    () => transport.run("cleanup.inspect", {}),
    /owned slice still present/,
  )
})

test("slice provisioning uses the independent lifecycle timeout", async () => {
  const { transport } = createCleanupTransport({
    startDelayMs: 20,
    timeouts: { sliceLifecycleMs: 5 },
  })
  await assert.rejects(
    () => transport.run("selkies.create", {
      runId: "run-timeout",
      kernelOwnedDefault: true,
      displayBackend: null,
      binding: {
        kernelId: "kernel-1",
        machineId: "machine-1",
        roomId: "room-1",
        environmentId: "environment-1",
      },
    }),
    /selkies\.create slice start timed out after 5ms/,
  )
  await transport.run("cleanup.perform", {})
})

test("destroy followed by final cleanup preserves the attachment evidence ledger", async () => {
  const { transport } = createCleanupTransport()
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await transport.run("selkies.create", {
    runId: "run-ledger",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  await transport.run("selkies.attach", {
    runId: "run-ledger",
    binding,
    client: "web",
    displayBackend: "selkies",
  })
  await transport.run("selkies.destroy", { sliceId: "slice-1" })
  await transport.run("cleanup.perform", {})
  const inspection = await transport.run("cleanup.inspect", {})

  assert.deepEqual(inspection.owned.attachmentIds, ["attachment-1"])
  assert.deepEqual(inspection.owned.detachedAttachmentIds, ["attachment-1"])
  assert.equal(inspection.zeroResidue, true)
})

test("cleanup reconciles partial detach acknowledgement loss before retrying", async () => {
  const { transport, detachAttempts } = createCleanupTransport({
    detachAckLossAttachmentId: "attachment-2",
  })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await transport.run("selkies.create", {
    runId: "run-detach-retry",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  for (const client of ["web", "local_tui"]) {
    await transport.run("selkies.attach", {
      runId: "run-detach-retry",
      binding,
      client,
      displayBackend: "selkies",
    })
  }

  await assert.rejects(
    () => transport.run("cleanup.perform", {}),
    /simulated detach acknowledgement loss: attachment-2/,
  )
  assert.deepEqual(detachAttempts, ["attachment-1", "attachment-2"])

  await transport.run("cleanup.perform", {})
  assert.deepEqual(detachAttempts, ["attachment-1", "attachment-2"])
  const inspection = await transport.run("cleanup.inspect", {})
  assert.deepEqual(inspection.owned.attachmentIds, ["attachment-1", "attachment-2"])
  assert.deepEqual(inspection.owned.detachedAttachmentIds, ["attachment-1", "attachment-2"])
  assert.equal(inspection.zeroResidue, true)
})

test("cleanup inspection rejects missing and malformed managed residue metrics", async () => {
  const base = {
    zeroResidue: true,
    managedMachines: 0,
    rooms: 0,
    environments: 0,
    processes: 0,
    listeners: 0,
    containers: 0,
    profiles: 0,
    activeTargets: 0,
    temporaryFiles: 0,
    retainedEvidenceLeakCount: 0,
    resources: { rssDeltaBytes: 0, diskDeltaBytes: 0 },
  }
  for (const [label, mutation] of [
    ["missing counter", ({ processes: _ignored, ...rest }) => rest],
    ["negative counter", (value) => ({ ...value, processes: -1 })],
    ["non-numeric counter", (value) => ({ ...value, processes: "0" })],
    ["missing resources", ({ resources: _ignored, ...rest }) => rest],
    ["non-finite delta", (value) => ({ ...value, resources: { ...value.resources, rssDeltaBytes: Infinity } })],
  ]) {
    const { transport } = createCleanupTransport({
      cleanupInspector: async () => mutation(base),
    })
    await transport.run("selkies.create", {
      runId: `run-${label.replaceAll(" ", "-")}`,
      kernelOwnedDefault: true,
      displayBackend: null,
      binding: {
        kernelId: "kernel-1",
        machineId: "machine-1",
        roomId: "room-1",
        environmentId: "environment-1",
      },
    })
    await transport.run("cleanup.perform", {})
    await assert.rejects(
      () => transport.run("cleanup.inspect", {}),
      /must be a finite non-negative|must contain finite non-negative deltas/,
      label,
    )
  }
})
