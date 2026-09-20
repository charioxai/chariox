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

test("factory uses an injected kernel-client seam without importing dist", async () => {
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
        },
        displayApi: { openSelkiesDisplayStream() {} },
        webSocket: {},
      },
    })
    assert.equal(transport.targetId, "machine-1")
    await transport.close()
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
    memory: { totalBytes: 8_000, availableBytes: 4_000 },
    disk: { totalBytes: 16_000, availableBytes: 12_000 },
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
})

test("managed resource telemetry can use the released daemon-health public path", async () => {
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: {
      async send(request) {
        assert.deepEqual(request, { GetDaemonHealth: null })
        return {
          DaemonHealth: {
            projection: {
              process: { process_id: 7, current_resident_set_bytes: 128 },
            },
          },
        }
      },
    },
    requestApi: {
      ...moduleRequestApi,
      getDaemonHealthRequest() { return { GetDaemonHealth: null } },
    },
    targetKernelRef: "kernel-1",
    targetMachineRef: "machine-1",
  })
  const result = await transport.collectManagedTargetResourceSnapshot({ phase: "health" })
  assert.deepEqual(result.telemetry, {
    scope: "managed-target",
    authoritative: true,
    targetId: "machine-1",
    source: "kernel-daemon-health",
  })
  assert.equal(result.process.rssBytes, 128)
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

test("managed resource telemetry has a finite timeout", async () => {
  const transport = createTelemetryTransport(new Promise(() => {}), {
    timeouts: { resourceTelemetryMs: 5 },
  })
  await assert.rejects(
    () => transport.collectManagedTargetResourceSnapshot({ phase: "during" }),
    /resource telemetry timed out after 5ms/,
  )
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

test("persistence descriptions preserve exact save/remove/restore argv and receipts", async () => {
  const evidence = persistenceEvidence()
  const transport = createManagedBrowserComputerParityTransportFromPublicClient({
    client: { async send() { throw new Error("unexpected persistence public request") } },
    requestApi: moduleRequestApi,
    persistence: {
      async describe() { return evidence },
      async run({ plan, onPersistenceMutation }) {
        for (const mutation of plan.persistenceMutations) {
          await onPersistenceMutation?.({ phase: "before", mutation })
          await onPersistenceMutation?.({ phase: "after", mutation })
        }
        return plan
      },
    },
  })

  const described = await transport.describePersistenceMutations({ runId: "run-1" })
  assert.deepEqual(
    described.persistenceMutations.map(({ action, argv, request }) => ({ action, argv, request })),
    evidence.persistenceMutations.map(({ action, argv, request }) => ({ action, argv, request })),
  )
  assert.deepEqual(
    described.persistenceMutations.map(({ action, receipt }) => ({ action, receipt })),
    evidence.persistenceMutations.map(({ action, receipt }) => ({ action, receipt })),
  )

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

function createCleanupTransport({ deleteRemovesSlice = true } = {}) {
  let slicePresent = true
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
  const requestApi = {
    ...moduleRequestApi,
    createSliceRequest(options) { return { CreateSlice: options } },
    bindRoomEnvironmentSliceRequest(sessionId, sliceId) {
      return { BindRoomEnvironmentSlice: { session_id: sessionId, slice_ref: sliceId } }
    },
    startSliceRequest(sliceId) { return { StartSlice: { slice_ref: sliceId } } },
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
  }
  const client = {
    async send(request) {
      if (Object.hasOwn(request, "CreateSlice")) {
        return { SliceCreated: { slice: { ...slice, status: "stopped", worker_kernel_id: null, worker_machine_id: null, environment_session_id: null, session_id: null } } }
      }
      if (Object.hasOwn(request, "BindRoomEnvironmentSlice")) {
        return { RoomEnvironmentSlice: { binding: { session_id: "room-1", slice_id: "slice-1", owner_kernel_id: "kernel-1", worker_kernel_ref: "worker-ref-1" } } }
      }
      if (Object.hasOwn(request, "StartSlice") || Object.hasOwn(request, "GetSlice")) {
        return Object.hasOwn(request, "StartSlice")
          ? { SliceStarted: { slice } }
          : { Slice: { slice } }
      }
      if (Object.hasOwn(request, "RelayStatus")) {
        return { RelayStatus: { status: { configured: true, connected: true, daemon_id: "kernel-1", machine_id: "machine-1" } } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentState")) {
        return { RoomEnvironmentState: { environment: { session_id: "room-1", environment_id: "environment-1" } } }
      }
      if (Object.hasOwn(request, "ListSessions")) {
        return { SessionsListed: { sessions: [{ id: "room-1" }] } }
      }
      if (Object.hasOwn(request, "ListSlices")) {
        return { SlicesListed: { slices: slicePresent ? [slice] : [] } }
      }
      if (Object.hasOwn(request, "GetRoomEnvironmentResourceInventory")) {
        return { RoomEnvironmentResourceInventory: { inventory: {
          session_id: "room-1",
          environment_id: "environment-1",
          slice_id: "slice-1",
          browser_ids: ["browser-1"],
          profile_ids: ["profile-1"],
        } } }
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
    }),
    client,
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
  assert.match(inspection.unsupportedChecks.join(" "), /global process\/listener\/container/)
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
