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
    cpuPercent: 5,
    cpuSampleWindowMs: 1_000,
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

  const withoutCpuPercent = managedTelemetry({ cpuPercent: undefined })
  await assert.rejects(
    () => createTelemetryTransport(withoutCpuPercent).collectManagedTargetResourceSnapshot({ phase: "during" }),
    /resource telemetry cpuPercent must be a finite non-negative number/,
  )

  const malformedCpuSampleWindow = managedTelemetry({ cpuSampleWindowMs: "1000" })
  await assert.rejects(
    () => createTelemetryTransport(malformedCpuSampleWindow).collectManagedTargetResourceSnapshot({ phase: "during" }),
    /resource telemetry cpuSampleWindowMs must be a finite non-negative number/,
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
          heartbeat_age_ms: 1,
          relay_peer_protocol_version: 335,
          relay_version: "relay-test",
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
    protocolApi: { LOCAL_DAEMON_PROTOCOL_VERSION: 335 },
    parityConfig: { expected: { roomId: "room-1", environmentId: "environment-1" } },
  })
}

test("compatibility preflight rejects stale and too-new kernel protocol constants before target inspection", async () => {
  for (const protocol of [322, 334]) {
    const transport = createCompatibilityTransport(protocol)
    await assert.rejects(
      () => transport.assertCompatibilityPreflight(),
      /requires released kernel protocol 335|observed 322|observed 334/,
      `protocol ${protocol}`,
    )
  }
})

test("compatibility preflight accepts the protocol 335 telemetry expectation", async () => {
  const transport = createCompatibilityTransport(335)
  const compatibility = await transport.assertCompatibilityPreflight()
  assert.equal(compatibility.protocol.kernel, 335)
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

function persistenceSlice(status) {
  return {
    id: "slice-1",
    status,
    environment_session_id: "room-1",
    environment_id: "environment-1",
  }
}

function persistenceObservation(status) {
  const observation = {
    slice: persistenceSlice(status),
  }
  if (status !== "stopped") {
    observation.inventory = {
      session_id: "room-1",
      environment_id: "environment-1",
      slice_id: "slice-1",
      browser_ids: ["browser-1"],
      profile_ids: ["profile-1"],
    }
  }
  return observation
}

function persistenceRequests() {
  return {
    save: { SaveSliceState: { slice_ref: "slice-1", mode: "shutdown", scope: "this_slice" } },
    remove: { StopSlice: { slice_ref: "slice-1" } },
    restore: { StartSlice: { slice_ref: "slice-1" } },
  }
}

function persistenceResponseForRequest(request) {
  const variant = Object.keys(request ?? {})[0]
  const savedState = {
    id: "saved-state-1",
    source_slice_id: "slice-1",
    home_archive_path: "/tmp/managed-parity-slice-state.tar",
  }
  if (variant === "SaveSliceState") {
    return { SliceStateSaved: { slice: persistenceSlice("stopped"), state: savedState } }
  }
  if (variant === "StopSlice") {
    return { SliceStopped: { slice: persistenceSlice("stopped") } }
  }
  if (variant === "StartSlice") {
    return { SliceStarted: { slice: persistenceSlice("running") } }
  }
  throw new Error(`unexpected persistence request: ${JSON.stringify(request)}`)
}

function persistenceArgv(action, request) {
  const variant = Object.keys(request)[0]
  const payload = request[variant]
  return [
    "kernel",
    variant,
    payload.slice_ref,
    ...(action === "save" ? [`mode=${payload.mode}`, `scope=${payload.scope}`] : []),
  ]
}

function persistenceEvidence() {
  const requests = persistenceRequests()
  const savedState = {
    id: "saved-state-1",
    source_slice_id: "slice-1",
    home_archive_path: "/tmp/managed-parity-slice-state.tar",
  }
  const responseSlice = (status) => persistenceSlice(status)
  const definitions = [
    {
      action: "save",
      before: persistenceObservation("running"),
      after: persistenceObservation("stopped"),
      response: {
        variant: "SliceStateSaved",
        payload: { slice: responseSlice("stopped"), state: savedState },
      },
      receipt: {
        ok: true,
        id: savedState.id,
        action: "save",
        requestIdentity: JSON.stringify(requests.save),
        responseVariant: "SliceStateSaved",
        responseSliceId: "slice-1",
        savedStateId: savedState.id,
        archivePath: savedState.home_archive_path,
        authoritative: true,
      },
    },
    {
      action: "remove",
      before: persistenceObservation("stopped"),
      after: persistenceObservation("stopped"),
      response: {
        variant: "SliceStopped",
        payload: { slice: responseSlice("stopped") },
      },
      receipt: {
        ok: true,
        id: "remove-receipt-1",
        action: "remove",
        requestIdentity: JSON.stringify(requests.remove),
        responseVariant: "SliceStopped",
        responseSliceId: "slice-1",
        savedStateId: savedState.id,
        parentReceiptId: savedState.id,
        archivePath: savedState.home_archive_path,
        authoritative: true,
      },
    },
    {
      action: "restore",
      before: persistenceObservation("stopped"),
      after: persistenceObservation("running"),
      response: {
        variant: "SliceStarted",
        payload: { slice: responseSlice("running") },
      },
      receipt: {
        ok: true,
        id: "restore-receipt-1",
        action: "restore",
        requestIdentity: JSON.stringify(requests.restore),
        responseVariant: "SliceStarted",
        responseSliceId: "slice-1",
        savedStateId: savedState.id,
        parentReceiptId: "remove-receipt-1",
        archivePath: savedState.home_archive_path,
        authoritative: true,
      },
    },
  ]
  return {
    persistenceMutations: definitions.map((mutation) => {
      const request = requests[mutation.action]
      return {
        ...mutation,
        argv: persistenceArgv(mutation.action, request),
        request,
        requestIdentity: JSON.stringify(request),
        responseVariant: mutation.response.variant,
        checkpoints: {
          before: `before-docker-${mutation.action}`,
          after: `after-docker-${mutation.action}`,
        },
        ...(mutation.action === "remove"
          ? { saveReceipt: definitions[0].receipt }
          : mutation.action === "restore"
            ? { saveReceipt: definitions[0].receipt, removeReceipt: definitions[1].receipt }
            : {}),
      }
    }),
  }
}

function persistencePlan() {
  return {
    persistenceMutations: persistenceEvidence().persistenceMutations.map((mutation) => {
      const {
        before,
        after,
        response,
        receipt,
        saveReceipt,
        removeReceipt,
        ...planMutation
      } = mutation
      return planMutation
    }),
  }
}

async function sendPlannedPersistenceRequests(client, plan) {
  for (const mutation of plan.persistenceMutations) await client.send(mutation.request)
}

test("persistence plans remain immutable and execution returns post-mutation receipts", async () => {
  const evidence = persistenceEvidence()
  const plan = persistencePlan()
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send(request) { return persistenceResponseForRequest(request) } },
    requestApi: moduleRequestApi,
    persistence: {
      async describe() { return plan },
      async run({ plan: executionPlan, onPersistenceMutation, client }) {
        for (const mutation of evidence.persistenceMutations) {
          await onPersistenceMutation?.({ phase: "before", mutation })
          await client.send(mutation.request)
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
    ["kernel", "SaveSliceState", "slice-1", "mode=shutdown", "scope=this_slice"],
    ["kernel", "StopSlice", "slice-1"],
    ["kernel", "StartSlice", "slice-1"],
  ])
  assert.deepEqual(result.persistenceMutations.map((mutation) => mutation.request), [
    persistenceRequests().save,
    persistenceRequests().remove,
    persistenceRequests().restore,
  ])
  assert.ok(Object.isFrozen(described))
  assert.ok(Object.isFrozen(described.persistenceMutations[0]))
})

test("persistence rejects predeclared receipts and callback-only no-op execution", async () => {
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
    /must await exactly one successful planned public lifecycle request|requires an authoritative pre-mutation slice state/,
  )
})

test("persistence does not count a caught rejected lifecycle send as executed evidence", async () => {
  const evidence = persistenceEvidence()
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send(request) {
      if (Object.hasOwn(request, "StopSlice")) throw new Error("simulated rejected stop")
      return persistenceResponseForRequest(request)
    } },
    requestApi: moduleRequestApi,
    persistence: {
      async describe() { return persistencePlan() },
      async run({ plan, client }) {
        await client.send(plan.persistenceMutations[0].request)
        try {
          await client.send(plan.persistenceMutations[1].request)
        } catch {
          // A caught rejection must not become a lifecycle receipt.
        }
        await client.send(plan.persistenceMutations[2].request)
        return evidence
      },
    },
  })
  await assert.rejects(
    () => transport.run("selkies.persistence", {}),
    /must await exactly one successful planned public lifecycle request per mutation/,
  )
})

test("persistence rejects missing or reordered lifecycle operations", async () => {
  const missing = persistencePlan()
  missing.persistenceMutations.pop()
  const reordered = persistencePlan()
  reordered.persistenceMutations.reverse()
  for (const invalidPlan of [missing, reordered]) {
    const transport = createManagedBrowserComputerParityTransportFromPublicClient({
      client: { async send() {} },
      requestApi: moduleRequestApi,
      persistence: { async describe() { return invalidPlan }, async run() { throw new Error("must not execute") } },
    })
    await assert.rejects(
      () => transport.describePersistenceMutations({ runId: "run-invalid-plan" }),
      /exact save\/remove\/restore|must be save|must be remove|must be restore/,
    )
  }
})

test("persistence rejects request/response mismatch and stale saved-state identity", async () => {
  const requestMismatch = persistenceEvidence()
  requestMismatch.persistenceMutations[1].argv = ["kernel", "StopSlice", "foreign-slice"]
  const responseMismatch = persistenceEvidence()
  responseMismatch.persistenceMutations[1].response.payload.slice.id = "foreign-slice"
  const staleState = persistenceEvidence()
  staleState.persistenceMutations[2].receipt.savedStateId = "stale-state"
  const cases = [requestMismatch, responseMismatch, staleState]
  for (const invalidEvidence of cases) {
    const transport = createManagedBrowserComputerParityTransportFromPublicClient({
      client: { async send(request) { return persistenceResponseForRequest(request) } },
      requestApi: moduleRequestApi,
      persistence: {
        async describe() { return persistencePlan() },
        async run({ plan, client }) {
          await sendPlannedPersistenceRequests(client, plan)
          return invalidEvidence
        },
      },
    })
    await assert.rejects(
      () => transport.run("selkies.persistence", {}),
      /does not describe|does not match|not bound to its captured lifecycle response|foreign slice identity|returned a different slice identity|stale saved-state identity/,
    )
  }

  const staleFixture = createCleanupTransport({ staleSavedStateId: "stale-state" })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await staleFixture.transport.run("selkies.create", {
    runId: "run-stale-saved-state",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  await assert.rejects(
    () => staleFixture.transport.run("selkies.persistence", { binding }),
    /did not restore the authoritative saved state|stale saved-state identity/,
  )
})

test("production persistence defaults to authoritative slice save, stop, and restore requests", async () => {
  const { transport, client } = createCleanupTransport({ inventoryUnavailableWhenStopped: true })
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
  assert.equal(
    client.requests.filter((request) => Object.hasOwn(request, "GetRoomEnvironmentResourceInventory")).length > 0,
    true,
  )
})

test("factory ignores runbook Docker expectations and exposes the exact kernel lifecycle plan", async () => {
  const runbookConfig = {
    expected: { roomId: "room-1", environmentId: "environment-1" },
    browserComputerGuard: {
      resourceTelemetry: { mode: "managed-target" },
      preflight: { requiredMemoryBytes: 0, requiredDiskBytes: 0 },
    },
  }
  const { transport } = createCleanupTransport({
    parityConfig: runbookConfig,
    protocolApi: { LOCAL_DAEMON_PROTOCOL_VERSION: 335 },
    targetKernelRef: "kernel-1",
  })
  await transport.assertCompatibilityPreflight({ config: runbookConfig })
  await transport.run("selkies.create", {
    runId: "run-runbook-config",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding: {
      kernelId: "kernel-1",
      machineId: "machine-1",
      roomId: "room-1",
      environmentId: "environment-1",
    },
  })
  const plan = await transport.describePersistenceMutations({ runId: "run-runbook-config" })
  assert.deepEqual(plan.persistenceMutations.map(({ request }) => Object.keys(request)[0]), [
    "SaveSliceState", "StopSlice", "StartSlice",
  ])
  assert.ok(plan.persistenceMutations.every(({ argv }) => argv[0] === "kernel"))
  assert.equal(Object.hasOwn(runbookConfig.browserComputerGuard, "dockerPreconditions"), false)
  assert.ok(plan.persistenceMutations.every((mutation) => !Object.hasOwn(mutation, "receipt")))
})

test("persistence timeout and partial lifecycle failure do not retry mutations", async () => {
  const timeoutFixture = createCleanupTransport({ startDelayMs: 40, timeouts: { persistenceMs: 10 } })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await timeoutFixture.transport.run("selkies.create", {
    runId: "run-persistence-timeout",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  const startsBeforePersistence = timeoutFixture.requests.filter((request) => Object.hasOwn(request, "StartSlice")).length
  await assert.rejects(
    () => timeoutFixture.transport.run("selkies.persistence", { binding }),
    /persistence execution timed out|persistence restore timed out/,
  )
  assert.equal(
    timeoutFixture.requests.filter((request) => Object.hasOwn(request, "StartSlice")).length,
    startsBeforePersistence + 1,
  )
  assert.equal(timeoutFixture.isClientClosed(), true)
  assert.equal(timeoutFixture.isCloseReceiverCorrect(), true)
  assert.equal(timeoutFixture.isDelayedRequestRetired(), true)

  const partialFixture = createCleanupTransport({ persistenceFailureAction: "remove" })
  await partialFixture.transport.run("selkies.create", {
    runId: "run-persistence-partial",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  await assert.rejects(
    () => partialFixture.transport.run("selkies.persistence", { binding }),
    /simulated persistence failure: remove/,
  )
  assert.deepEqual(partialFixture.requests.filter((request) => [
    "SaveSliceState", "StopSlice", "StartSlice",
  ].some((variant) => Object.hasOwn(request, variant))).map((request) => Object.keys(request)[0]).slice(-2), [
    "SaveSliceState", "StopSlice",
  ])
})

function createCleanupTransport({
  deleteRemovesSlice = true,
  startDelayMs = 0,
  persistenceFailureAction = null,
  staleSavedStateId = null,
  persistenceResponseMismatch = null,
  detachAckLossAttachmentId = null,
  residualBrowserId = null,
  postDeleteEnvironmentResidue = false,
  cleanupInspector = null,
  operationAdapter = null,
  trackedProviderAgents = [],
  initialAgentIds = [],
  failDestroyAgentId = null,
  useSeparateScopedClient = false,
  reconnectClient = null,
  reconnectActionSnapshots = null,
  reconnectBrowserSnapshots = null,
  inventoryUnavailableWhenStopped = false,
  telemetrySamples = null,
  evidenceRoot = "/proc/managed-parity-m0-evidence-never-present",
  parityConfig = null,
  protocolApi = null,
  targetKernelRef = "worker-ref-1",
  timeouts,
} = {}) {
  let slicePresent = true
  let roomPresent = true
  let roomEnded = false
  let attachmentSequence = 0
  let ackLossInjected = false
  let destroyFailureInjected = false
  let reconnectHistoryCalls = 0
  let reconnectInventoryCalls = 0
  let reconnectMode = false
  const activeAgentIds = new Set(initialAgentIds)
  const activeAttachmentIds = new Set()
  const detachAttempts = []
  const requests = []
  let clientClosed = false
  let closeReceiverCorrect = false
  let pendingDelayedRequestReject = null
  let delayedRequestRetired = false
  const slice = {
    id: "slice-1",
    backend: "ssh_docker",
    display_mode: "headed",
    worker_kernel_ref: targetKernelRef,
    worker_kernel_id: "kernel-1",
    worker_machine_id: "machine-1",
    environment_session_id: "room-1",
    session_id: "room-1",
    status: "running",
    agent_ids: [...activeAgentIds],
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
    getRoomEnvironmentSliceRequest(sessionId) {
      return { GetRoomEnvironmentSlice: { session_id: sessionId } }
    },
    getRoomEnvironmentStateRequest(sessionId) {
      return { GetRoomEnvironmentState: { session_id: sessionId } }
    },
    getKernelResourceTelemetryRequest() { return { GetKernelResourceTelemetry: null } },
    LOCAL_DAEMON_PROTOCOL_VERSION: 335,
    relayStatusRequest() { return { RelayStatus: null } },
    deleteSliceRequest(sliceId) { return { DeleteSlice: { slice_ref: sliceId } } },
    destroyAgentRequest(sessionId, agentId) {
      return { DestroyAgent: { session_id: sessionId, agent_id: agentId } }
    },
    listAgentsRequest(sessionId) { return { ListAgents: { session_id: sessionId } } },
    listRoomEnvironmentActionHistoryRequest(sessionId, before, limit) {
      return { ListRoomEnvironmentActionHistory: { session_id: sessionId, before, limit } }
    },
    endSessionRequest(sessionId) { return { EndSession: { session_id: sessionId } } },
    deleteSessionRequest(sessionId) { return { DeleteSession: { session_ref: sessionId, workspace_id: null } } },
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
        return { RoomEnvironmentSlice: { binding: {
          session_id: "room-1",
          slice_id: "slice-1",
          owner_kernel_id: "kernel-1",
          worker_kernel_ref: targetKernelRef,
        } } }
      }
      if (Object.hasOwn(request, "StartSlice") || Object.hasOwn(request, "GetSlice")) {
        if (Object.hasOwn(request, "StartSlice") && startDelayMs > 0) {
          await new Promise((resolve, reject) => {
            const timer = setTimeout(() => {
              pendingDelayedRequestReject = null
              resolve()
            }, startDelayMs)
            pendingDelayedRequestReject = (error) => {
              clearTimeout(timer)
              pendingDelayedRequestReject = null
              delayedRequestRetired = true
              reject(error)
            }
          })
        }
        if (Object.hasOwn(request, "StartSlice")) slice.status = "running"
        const observedSlice = persistenceResponseMismatch === "restore"
          ? { ...slice, id: "foreign-slice" }
          : slice
        return Object.hasOwn(request, "StartSlice")
          ? { SliceStarted: { slice: observedSlice } }
          : { Slice: { slice: observedSlice } }
      }
      if (Object.hasOwn(request, "SaveSliceState")) {
        if (persistenceFailureAction === "save") {
          throw new Error("simulated persistence failure: save")
        }
        savedState = {
          id: "saved-state-1",
          source_slice_id: "slice-1",
          home_archive_path: "/tmp/managed-parity-slice-state.tar",
        }
        if (request.SaveSliceState.mode === "shutdown") slice.status = "stopped"
        const observedState = persistenceResponseMismatch === "save"
          ? { ...savedState, source_slice_id: "foreign-slice" }
          : savedState
        return { SliceStateSaved: { slice, state: observedState } }
      }
      if (Object.hasOwn(request, "StopSlice")) {
        if (persistenceFailureAction === "remove") {
          throw new Error("simulated persistence failure: remove")
        }
        slice.status = "stopped"
        const observedSlice = persistenceResponseMismatch === "remove"
          ? { ...slice, id: "foreign-slice" }
          : slice
        return { SliceStopped: { slice: observedSlice } }
      }
      if (Object.hasOwn(request, "GetSliceStateStatus")) {
        const observedState = staleSavedStateId
          ? { ...savedState, id: staleSavedStateId }
          : savedState
        return { SliceStateStatus: { slice, state: observedState } }
      }
      if (Object.hasOwn(request, "RelayStatus")) {
        return { RelayStatus: { status: {
          configured: true,
          connected: true,
          daemon_id: "kernel-1",
          machine_id: "machine-1",
          heartbeat_age_ms: 0,
          relay_peer_protocol_version: 335,
          relay_version: "fixture-relay",
        } } }
      }
      if (Object.hasOwn(request, "GetKernelResourceTelemetry")) {
        return { KernelResourceTelemetry: { snapshot: managedTelemetry() } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentState")) {
        const environmentActive = roomPresent && (slicePresent || postDeleteEnvironmentResidue)
        return { RoomEnvironmentState: { environment: {
          session_id: "room-1",
          environment_id: "environment-1",
          runtime_generation: 1,
          lifecycle: environmentActive ? "ready" : "stopped",
          health: ["browser_controller", "browser", "desktop", "streamer"].map((component) => ({
            component,
            state: environmentActive ? "ready" : "unavailable",
            diagnostic_code: null,
          })),
          viewport: { revision: 1 },
          tabs: [],
          actions: [],
          input_ownership: [],
          pending_input_takeovers: [],
        } } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentSlice")) {
        return { RoomEnvironmentSlice: { binding: slicePresent && roomPresent
          ? { session_id: "room-1", slice_id: "slice-1", owner_kernel_id: "kernel-1", worker_kernel_ref: targetKernelRef }
          : null } }
      }
      if (Object.hasOwn(request, "ListSessions")) {
        return { SessionsListed: { sessions: roomPresent
          ? [{ id: "room-1", status: roomEnded ? "ended" : "active", attachment_ids: [...activeAttachmentIds] }]
          : [] } }
      }
      if (Object.hasOwn(request, "ListSlices")) {
        return { SlicesListed: { slices: slicePresent ? [{ ...slice, agent_ids: [...activeAgentIds] }] : [] } }
      }
      if (Object.hasOwn(request, "ListAgents")) {
        return { AgentsListed: { agents: [...activeAgentIds].map((id) => ({
          id,
          session_id: "room-1",
          provider: "codex",
          model: "gpt-test",
        })) } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentResourceInventory")) {
        if (inventoryUnavailableWhenStopped && slice.status === "stopped") {
          throw new Error("live worker inventory unavailable while stopped")
        }
        const browserIds = reconnectMode && Array.isArray(reconnectBrowserSnapshots)
          ? reconnectBrowserSnapshots[Math.min(reconnectInventoryCalls++, reconnectBrowserSnapshots.length - 1)]
          : slicePresent ? ["browser-1"] : (residualBrowserId ? [residualBrowserId] : [])
        return { RoomEnvironmentResourceInventory: { inventory: {
          session_id: "room-1",
          environment_id: "environment-1",
          slice_id: "slice-1",
          browser_ids: browserIds,
          profile_ids: slicePresent ? ["profile-1"] : (residualBrowserId ? ["profile-1"] : []),
        } } }
      }
      if (Object.hasOwn(request, "ListRoomEnvironmentActionHistory")) {
        reconnectMode = true
        const actionIds = Array.isArray(reconnectActionSnapshots)
          ? reconnectActionSnapshots[Math.min(reconnectHistoryCalls++, reconnectActionSnapshots.length - 1)]
          : ["action-1"]
        return { RoomEnvironmentActionHistoryListed: {
          page: { actions: actionIds.map((actionId) => ({ action_id: actionId })) },
        } }
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
      if (Object.hasOwn(request, "DestroyAgent")) {
        const agentId = request.DestroyAgent.agent_id
        if (agentId === failDestroyAgentId && !destroyFailureInjected) {
          destroyFailureInjected = true
          throw new Error(`simulated agent retirement failure: ${agentId}`)
        }
        activeAgentIds.delete(agentId)
        return { AgentDestroyed: { agent: { id: agentId, session_id: "room-1" } } }
      }
      if (Object.hasOwn(request, "EndSession")) {
        roomEnded = true
        return { SessionEnded: { session: { id: "room-1", status: "ended" } } }
      }
      if (Object.hasOwn(request, "DeleteSession")) {
        roomPresent = false
        return { SessionDeleted: { session: { id: "room-1", status: "ended" } } }
      }
      throw new Error(`unexpected cleanup request: ${JSON.stringify(request)}`)
    },
    async close() {
      closeReceiverCorrect = this === client
      clientClosed = true
      pendingDelayedRequestReject?.(new Error("delayed request retired by client close"))
    },
    async destroy() {
      closeReceiverCorrect = this === client
      clientClosed = true
      pendingDelayedRequestReject?.(new Error("delayed request retired by client destroy"))
    },
  }
  const providerAgentQueue = [...trackedProviderAgents]
  const providerOperationAdapter = operationAdapter ?? (providerAgentQueue.length > 0
    ? {
      "selkies.browser": async () => {
        const agentId = providerAgentQueue.shift()
        if (agentId) activeAgentIds.add(agentId)
        return agentId ? { agentId } : {}
      },
      "selkies.computer": async () => {
        const agentId = providerAgentQueue.shift()
        if (agentId) activeAgentIds.add(agentId)
        return agentId ? { agentId } : {}
      },
    }
    : null)
  let scopedClosed = false
  const scopedClient = {
    async send(request) {
      if (scopedClosed) throw new Error("scoped client disconnected")
      return client.send(request)
    },
    async close() {
      scopedClosed = true
    },
  }
  const transportClient = useSeparateScopedClient ? scopedClient : client
  return {
    transport: createManagedBrowserComputerParityTransportFromPublicClient({
      client: transportClient,
      requestApi,
      targetKernelRef,
      targetMachineRef: "machine-1",
      cleanupInspector,
      operationAdapter: providerOperationAdapter,
      displayClient: client,
      identityClient: transportClient,
      reconnectClient,
      resourceTelemetry: telemetrySamples ?? (() => managedTelemetry()),
      parityConfig,
      protocolApi,
      timeouts,
      evidenceRoot,
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
    scopedClient,
    isScopedClosed: () => scopedClosed,
    isClientClosed: () => clientClosed,
    isCloseReceiverCorrect: () => closeReceiverCorrect,
    isDelayedRequestRetired: () => delayedRequestRetired,
    detachAttempts,
    requests,
    activeAgentIds,
  }
}

test("cleanup removes the owned Room after slice deletion and proves no public residue", async () => {
  const { transport, requests } = createCleanupTransport()
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

  const cleanupLifecycle = requests
    .filter((request) => Object.hasOwn(request, "DeleteSlice")
      || Object.hasOwn(request, "GetRoomEnvironmentSlice")
      || Object.hasOwn(request, "EndSession")
      || Object.hasOwn(request, "DeleteSession"))
    .map((request) => Object.keys(request)[0])
  assert.deepEqual(cleanupLifecycle, [
    "DeleteSlice", "GetRoomEnvironmentSlice", "EndSession", "DeleteSession",
  ])
  assert.equal(inspection.zeroResidue, true)
  assert.equal(inspection.owned.sliceId, "slice-1")
  assert.equal(inspection.owned.deleted, true)
  assert.equal(inspection.ownedSliceCount, 0)
  assert.equal(inspection.ownedAttachmentResidueCount, 0)
  assert.deepEqual(inspection.resources, { rssDeltaBytes: 0, diskDeltaBytes: 0 })
  assert.deepEqual(inspection.unsupportedChecks, [])
})

test("cleanup retires Browser and Computer agents before authoritative DeleteSlice", async () => {
  const { transport, requests, activeAgentIds } = createCleanupTransport({
    trackedProviderAgents: ["agent-browser", "agent-computer"],
  })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await transport.run("selkies.create", {
    runId: "run-agents",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  await transport.run("selkies.browser", { binding })
  await transport.run("selkies.computer", { binding })
  await transport.run("selkies.destroy", { sliceId: "slice-1" })

  const retirementAndDelete = requests
    .filter((request) => Object.hasOwn(request, "DestroyAgent") || Object.hasOwn(request, "DeleteSlice"))
    .map((request) => Object.keys(request)[0])
  assert.deepEqual(retirementAndDelete, ["DestroyAgent", "DestroyAgent", "DeleteSlice"])
  assert.deepEqual(activeAgentIds, new Set())
  await transport.run("cleanup.perform", {})
})

test("cleanup retries partial agent retirement before deleting the slice", async () => {
  const { transport, requests, activeAgentIds } = createCleanupTransport({
    trackedProviderAgents: ["agent-browser", "agent-computer"],
    failDestroyAgentId: "agent-computer",
  })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await transport.run("selkies.create", {
    runId: "run-agent-retry",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  await transport.run("selkies.browser", { binding })
  await transport.run("selkies.computer", { binding })
  await assert.rejects(
    () => transport.run("selkies.destroy", { sliceId: "slice-1" }),
    /simulated agent retirement failure: agent-computer/,
  )
  assert.deepEqual(
    requests
      .filter((request) => Object.hasOwn(request, "DestroyAgent"))
      .map((request) => request.DestroyAgent.agent_id),
    ["agent-browser", "agent-computer"],
  )
  assert.deepEqual(activeAgentIds, new Set(["agent-computer"]))

  await transport.run("selkies.destroy", { sliceId: "slice-1" })
  assert.deepEqual(
    requests
      .filter((request) => Object.hasOwn(request, "DestroyAgent"))
      .map((request) => request.DestroyAgent.agent_id),
    ["agent-browser", "agent-computer", "agent-computer"],
  )
  assert.equal(requests.filter((request) => Object.hasOwn(request, "DeleteSlice")).length, 1)
  await transport.run("cleanup.perform", {})
})

test("selkies.reconnect observes scoped disconnect before reconnect and counts duplicate identities", async () => {
  let reconnectCalls = 0
  let replacementClosed = false
  const replacement = {
    async send(request) {
      if (Object.hasOwn(request, "RelayStatus")) {
        return { RelayStatus: { status: {
          configured: true,
          connected: true,
          daemon_id: "kernel-1",
          machine_id: "machine-1",
        } } }
      }
      throw new Error(`unexpected replacement request: ${JSON.stringify(request)}`)
    },
    async close() {
      replacementClosed = true
    },
  }
  const fixture = createCleanupTransport({
    useSeparateScopedClient: true,
    reconnectActionSnapshots: [["action-1"], ["action-1", "action-1"]],
    reconnectBrowserSnapshots: [["browser-1"], ["browser-1", "browser-1"]],
    reconnectClient: async () => {
      reconnectCalls += 1
      assert.equal(fixture.isScopedClosed(), true)
      return replacement
    },
  })
  const binding = {
    kernelId: "kernel-1",
    machineId: "machine-1",
    roomId: "room-1",
    environmentId: "environment-1",
  }
  await fixture.transport.run("selkies.create", {
    runId: "run-reconnect",
    kernelOwnedDefault: true,
    displayBackend: null,
    binding,
  })
  const result = await fixture.transport.run("selkies.reconnect", {
    binding,
    fault: "relay_disconnect",
  })

  assert.equal(reconnectCalls, 1)
  assert.equal(result.disconnectObserved, true)
  assert.equal(result.reconnected, true)
  assert.equal(result.duplicateActions, 1)
  assert.equal(result.duplicateBrowsers, 1)
  assert.deepEqual(result.beforeActionIds, ["action-1"])
  assert.deepEqual(result.afterActionIds, ["action-1", "action-1"])
  assert.deepEqual(result.beforeBrowserIds, ["browser-1"])
  assert.deepEqual(result.afterBrowserIds, ["browser-1", "browser-1"])
  await fixture.transport.run("cleanup.perform", {})
  await replacement.close()
  assert.equal(replacementClosed, true)
})

test("cleanup.inspect fails closed on authoritative post-delete environment residue", async () => {
  const { transport } = createCleanupTransport({ postDeleteEnvironmentResidue: true })
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
    /found residual resources from authoritative post-delete inventories/,
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
  await assert.rejects(
    () => transport.run("cleanup.perform", {}),
    /left the owned slice present/,
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
      /must be a finite non-negative|must contain finite non-negative deltas|must be a finite number/,
      label,
    )
  }
})
