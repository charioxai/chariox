import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import { chmod, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { createServer } from "node:net"
import { once } from "node:events"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { createSliceDiskQuotaAllocator } from "./slice-disk-quota-allocator.mjs"
import {
  hasMatchingSliceDiskQuotaUnboundedProof,
  runWithSliceDiskQuotaAdmission,
  writeSliceDiskQuotaUnboundedProof,
} from "./slice-disk-quota-admission.mjs"
import { requestSliceDiskQuota } from "./slice-disk-quota-client.mjs"
import {
  SLICE_DISK_QUOTA_PROJECT_ID_MIN,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
} from "./slice-disk-quota-contract.mjs"
import { createSliceDiskQuotaCoordinator } from "./slice-disk-quota-coordinator.mjs"
import { handleSliceDiskQuotaConnection } from "./slice-disk-quota-service.mjs"
import { createFileSliceDiskQuotaStateStore } from "./slice-disk-quota-state-store.mjs"

const linuxTest = process.platform === "linux" ? test : test.skip
const LIMITS = { writableLayerBytes: 1024 * 1024, persistentHomeBytes: 2 * 1024 * 1024 }

function identity(suffix = "one") {
  const containerName = `chariox-slice-coord-${suffix}`
  return {
    ownerKernelId: "kernel-coordinator",
    ownerMachineId: "machine-coordinator",
    sliceId: `slice-${suffix}`,
    containerName,
    homeVolumeName: `${containerName}-home`,
  }
}

function binding(homeVolumeName, containerId = "a".repeat(64)) {
  return {
    containerId,
    homeVolumeCreatedAt: "2026-09-27T12:00:00Z",
    homeVolumeDevice: "2049",
    homeVolumeDriver: "local",
    homeVolumeInode: "1337",
    homeVolumeMountpoint: `/var/lib/chariox-docker/data/volumes/${homeVolumeName}/_data`,
    homeVolumeName,
    homeVolumeScope: "local",
  }
}

function emptyState() {
  return {
    schemaVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    nextProjectId: SLICE_DISK_QUOTA_PROJECT_ID_MIN,
    reservations: {},
  }
}

async function setup(context, { initialState = emptyState(), save } = {}) {
  const root = await mkdtemp(join(tmpdir(), "slice-disk-quota-coordination-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const statePath = join(root, "state", "reservations.json")
  const baseStore = createFileSliceDiskQuotaStateStore(statePath)
  if (initialState !== undefined) baseStore.save(initialState)
  const stateStore = save ? {
    load: () => baseStore.load(),
    loadRequired: () => baseStore.loadRequired(),
    save,
  } : baseStore
  const paths = {
    statePath,
    coordinationRoot: join(root, "state", "coordination"),
    proofRoot: join(root, "state", "unbounded-proofs"),
    legacyProofRoot: join(root, "legacy-handle-state.unbounded-quota"),
  }
  const coordinator = createSliceDiskQuotaCoordinator({
    stateStore,
    ...paths,
    lockWaitMs: 1_500,
  })
  return { root, statePath, baseStore, stateStore, paths, coordinator }
}

function testAllocator(stateStore) {
  const backend = {
    probe: () => ({
      supported: true,
      totalBytes: 100 * 1024 ** 3,
      availableBytes: 90 * 1024 ** 3,
      hostTotalBytes: 200 * 1024 ** 3,
      hostAvailableBytes: 180 * 1024 ** 3,
    }),
    projectIdsInUse: () => [],
    projectUsageBytes: () => 0,
    confirmContainerAndVolumeRemoved: () => true,
    clearProjectQuota: () => {},
  }
  return createSliceDiskQuotaAllocator({ backend, stateStore })
}

async function startQuotaService(context, allocator, coordinator) {
  const root = await mkdtemp(join(tmpdir(), "slice-disk-quota-coordinator-socket-"))
  const socketPath = join(root, "allocator.sock")
  const server = createServer({ allowHalfOpen: true }, (socket) => {
    handleSliceDiskQuotaConnection(socket, allocator, { coordinator, requestTimeoutMs: 3_000 })
  })
  server.listen(socketPath)
  await once(server, "listening")
  context.after(async () => {
    await new Promise((resolve) => server.close(resolve))
    await rm(root, { recursive: true, force: true })
  })
  return socketPath
}

function reserveRequest(sliceIdentity) {
  return {
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    operation: "reserve",
    identity: sliceIdentity,
    limits: LIMITS,
  }
}

linuxTest("direct exported-client reserve revokes a captured receipt before durable commit", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity()
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  const replacementIdentity = {
    ...sliceIdentity,
    ownerMachineId: "machine-replaced",
    sliceId: "slice-replaced",
  }
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, (lock) => (
    fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
  ))

  const productionAllocator = testAllocator(fixture.stateStore)
  let receiptWasRevokedBeforeAllocator
  const allocator = {
    handle(request) {
      if (request.operation === "reserve") {
        receiptWasRevokedBeforeAllocator = !hasMatchingSliceDiskQuotaUnboundedProof(
          fixture.paths.proofRoot,
          sliceIdentity,
          sliceBinding,
        )
      }
      return productionAllocator.handle(request)
    },
  }
  const socketPath = await startQuotaService(context, allocator, fixture.coordinator)
  assert.deepEqual(await requestSliceDiskQuota(reserveRequest(replacementIdentity), { socketPath, requestTimeoutMs: 2_000 }), {
    reserved: true,
    projectIds: { writableLayer: SLICE_DISK_QUOTA_PROJECT_ID_MIN, persistentHome: SLICE_DISK_QUOTA_PROJECT_ID_MIN + 1 },
  })
  assert.equal(receiptWasRevokedBeforeAllocator, true)
  assert.equal(Object.keys(fixture.baseStore.loadRequired().reservations).length, 1)
  const restartedAllocator = testAllocator(createFileSliceDiskQuotaStateStore(fixture.statePath))
  assert.equal(restartedAllocator.handle({
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    operation: "status",
    identity: replacementIdentity,
  }).bounded, true)

  let started = false
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    assert.equal(await fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding), undefined)
    await assert.rejects(
      fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding),
      /reservation prevents an unbounded proof/,
    )
    await assert.rejects(runWithSliceDiskQuotaAdmission({
      containerName: sliceIdentity.containerName,
      quotaMarkerPresent: false,
      requestQuota: async () => { throw Object.assign(new Error("allocator offline"), { code: "ECONNREFUSED" }) },
      resolveUnboundedProof: () => fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding),
      run: () => { started = true },
    }), /allocator offline/)
  })
  assert.equal(started, false)
})

linuxTest("restarted coordinator ignores legacy receipts and uses strict shared durable state", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity("restart")
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  writeSliceDiskQuotaUnboundedProof(fixture.paths.legacyProofRoot, sliceIdentity, sliceBinding)

  const restarted = createSliceDiskQuotaCoordinator({
    stateStore: createFileSliceDiskQuotaStateStore(fixture.statePath),
    ...fixture.paths,
    lockWaitMs: 1_500,
  })
  await restarted.withContainerLock(sliceIdentity.containerName, async (lock) => {
    assert.equal(await restarted.resolveUnboundedProof(lock, sliceIdentity, sliceBinding), undefined)
    await assert.rejects(runWithSliceDiskQuotaAdmission({
      containerName: sliceIdentity.containerName,
      quotaMarkerPresent: false,
      requestQuota: async () => { throw Object.assign(new Error("allocator unavailable"), { code: "ENOENT" }) },
      resolveUnboundedProof: () => restarted.resolveUnboundedProof(lock, sliceIdentity, sliceBinding),
      run: () => assert.fail("legacy checkpoint receipt must not authorize rollout fallback"),
    }), /allocator unavailable/)

    await restarted.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
  })

  const afterRestart = createSliceDiskQuotaCoordinator({
    stateStore: createFileSliceDiskQuotaStateStore(fixture.statePath),
    ...fixture.paths,
    lockWaitMs: 1_500,
  })
  await afterRestart.withContainerLock(sliceIdentity.containerName, async (lock) => {
    const result = await runWithSliceDiskQuotaAdmission({
      containerName: sliceIdentity.containerName,
      quotaMarkerPresent: false,
      requestQuota: async () => { throw Object.assign(new Error("allocator unavailable"), { code: "ECONNREFUSED" }) },
      resolveUnboundedProof: () => afterRestart.resolveUnboundedProof(lock, sliceIdentity, sliceBinding),
      run: (quota, admission) => ({ quota, admission }),
    })
    assert.deepEqual(result, {
      quota: { bounded: false },
      admission: { source: "broker-proof", containerId: sliceBinding.containerId },
    })
  })
})

linuxTest("malformed, unreadable, or missing durable state cannot validate an offline receipt", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity("badstate")
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, (lock) => (
    fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
  ))

  await writeFile(fixture.statePath, "not-json\n")
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    await assert.rejects(fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding))
  })

  fixture.baseStore.save(emptyState())
  await chmod(fixture.statePath, 0o644)
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    await assert.rejects(fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding), /unsafe/)
  })

  await chmod(fixture.statePath, 0o600)
  await rm(fixture.statePath)
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    await assert.rejects(fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding), { code: "ENOENT" })
    await assert.rejects(fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding), { code: "ENOENT" })
  })
})

linuxTest("receipt capture rejects reservation identity and Docker container replacements", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity("replacement")
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    await fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
    const changedIdentity = { ...sliceIdentity, ownerMachineId: "machine-replaced" }
    await assert.rejects(
      fixture.coordinator.resolveUnboundedProof(lock, changedIdentity, sliceBinding),
      /identity is stale/,
    )
    assert.equal(await fixture.coordinator.resolveUnboundedProof(
      lock,
      sliceIdentity,
      binding(sliceIdentity.homeVolumeName, "b".repeat(64)),
    ), undefined)
  })
})

linuxTest("competing broker capture and actual service reserve share the container lock", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity("compete")
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  const allocator = testAllocator(fixture.stateStore)
  const serviceCoordinator = createSliceDiskQuotaCoordinator({
    stateStore: createFileSliceDiskQuotaStateStore(fixture.statePath),
    ...fixture.paths,
    lockWaitMs: 1_500,
  })
  let signalReserveQueued
  const reserveQueued = new Promise((resolve) => { signalReserveQueued = resolve })
  const observedCoordinator = {
    ...serviceCoordinator,
    runReservation(...args) {
      signalReserveQueued()
      return serviceCoordinator.runReservation(...args)
    },
  }
  const socketPath = await startQuotaService(context, allocator, observedCoordinator)
  let signalCaptured
  const captured = new Promise((resolve) => { signalCaptured = resolve })
  let releaseCapture
  const captureGate = new Promise((resolve) => { releaseCapture = resolve })
  const capture = fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    await fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
    signalCaptured()
    await captureGate
  })
  await captured
  const reserve = requestSliceDiskQuota(reserveRequest(sliceIdentity), { socketPath, requestTimeoutMs: 2_500 })
  void reserve.catch(() => {})
  try {
    let queueDeadline
    const didQueue = await Promise.race([
      reserveQueued.then(() => true),
      new Promise((resolve) => { queueDeadline = setTimeout(() => resolve(false), 1_000) }),
    ])
    clearTimeout(queueDeadline)
    assert.equal(didQueue, true)
    await new Promise((resolve) => setTimeout(resolve, 40))
    assert.equal(Object.keys(fixture.baseStore.loadRequired().reservations).length, 0)
  } finally {
    releaseCapture()
  }
  await capture
  await reserve

  assert.equal(Object.keys(fixture.baseStore.loadRequired().reservations).length, 1)
  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    assert.equal(await fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding), undefined)
  })
})

linuxTest("different container reserves serialize aggregate durable-state writes", async (context) => {
  const fixture = await setup(context)
  const allocator = testAllocator(fixture.stateStore)
  const socketPath = await startQuotaService(context, allocator, fixture.coordinator)
  const results = await Promise.all([
    requestSliceDiskQuota(reserveRequest(identity("parallel-a")), { socketPath, requestTimeoutMs: 2_500 }),
    requestSliceDiskQuota(reserveRequest(identity("parallel-b")), { socketPath, requestTimeoutMs: 2_500 }),
  ])
  assert.equal(results.length, 2)
  assert.equal(Object.keys(fixture.baseStore.loadRequired().reservations).length, 2)
})

linuxTest("aggregate-state lock serializes different container coordinators", async (context) => {
  const fixture = await setup(context)
  const secondCoordinator = createSliceDiskQuotaCoordinator({
    stateStore: createFileSliceDiskQuotaStateStore(fixture.statePath),
    ...fixture.paths,
    lockWaitMs: 1_500,
  })
  let signalFirstHeld
  const firstHeld = new Promise((resolve) => { signalFirstHeld = resolve })
  let releaseFirst
  const firstGate = new Promise((resolve) => { releaseFirst = resolve })
  let secondAttempted = false
  let secondEntered = false
  const first = fixture.coordinator.withContainerLock("chariox-slice-coord-state-a", async (lock) => {
    await fixture.coordinator.withStateLock(lock, async () => {
      signalFirstHeld()
      await firstGate
    })
  })
  await firstHeld
  const second = secondCoordinator.withContainerLock("chariox-slice-coord-state-b", async (lock) => {
    secondAttempted = true
    await secondCoordinator.withStateLock(lock, () => { secondEntered = true })
  })
  try {
    let stateWaitDeadline
    const attempted = await Promise.race([
      (async () => {
        while (!secondAttempted) await new Promise((resolve) => setTimeout(resolve, 1))
        return true
      })(),
      new Promise((resolve) => { stateWaitDeadline = setTimeout(() => resolve(false), 1_000) }),
    ])
    clearTimeout(stateWaitDeadline)
    assert.equal(attempted, true)
    await new Promise((resolve) => setTimeout(resolve, 40))
    assert.equal(secondEntered, false)
  } finally {
    releaseFirst()
  }
  await Promise.all([first, second])
  assert.equal(secondEntered, true)
})

linuxTest("actual unbounded start keeps its container lock through run before a service reserve", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity("start-reserve")
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  const allocator = testAllocator(fixture.stateStore)
  const serviceCoordinator = createSliceDiskQuotaCoordinator({
    stateStore: createFileSliceDiskQuotaStateStore(fixture.statePath),
    ...fixture.paths,
    lockWaitMs: 1_500,
  })
  let signalReserveAttempt
  const reserveAttempted = new Promise((resolve) => { signalReserveAttempt = resolve })
  const observedServiceCoordinator = {
    ...serviceCoordinator,
    runReservation(...args) {
      signalReserveAttempt()
      return serviceCoordinator.runReservation(...args)
    },
  }
  const socketPath = await startQuotaService(context, allocator, observedServiceCoordinator)

  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, (lock) => (
    fixture.coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
  ))
  let signalRunEntered
  const runEntered = new Promise((resolve) => { signalRunEntered = resolve })
  let releaseRun
  const runGate = new Promise((resolve) => { releaseRun = resolve })
  const start = fixture.coordinator.withContainerLock(sliceIdentity.containerName, (lock) => (
    runWithSliceDiskQuotaAdmission({
      containerName: sliceIdentity.containerName,
      quotaMarkerPresent: false,
      requestQuota: (request) => requestSliceDiskQuota(request, { socketPath, requestTimeoutMs: 2_500 }),
      resolveUnboundedProof: () => fixture.coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding),
      run: async (quota, admission) => {
        assert.deepEqual(quota, { bounded: false })
        assert.deepEqual(admission, { source: "allocator", containerId: undefined })
        signalRunEntered()
        await runGate
        return { status: "started" }
      },
    })
  ))
  let startDeadline
  const startReachedRun = await Promise.race([
    runEntered.then(() => true),
    start.then(() => false, (error) => { throw error }),
    new Promise((resolve) => { startDeadline = setTimeout(() => resolve(false), 3_500) }),
  ])
  clearTimeout(startDeadline)
  assert.equal(startReachedRun, true)

  const reserve = requestSliceDiskQuota(reserveRequest(sliceIdentity), { socketPath, requestTimeoutMs: 2_500 })
  void reserve.catch(() => {})
  try {
    let queueDeadline
    const didAttempt = await Promise.race([
      reserveAttempted.then(() => true),
      new Promise((resolve) => { queueDeadline = setTimeout(() => resolve(false), 1_000) }),
    ])
    clearTimeout(queueDeadline)
    assert.equal(didAttempt, true)
    await new Promise((resolve) => setTimeout(resolve, 40))
    assert.equal(Object.keys(fixture.baseStore.loadRequired().reservations).length, 0)
    assert.equal(hasMatchingSliceDiskQuotaUnboundedProof(fixture.paths.proofRoot, sliceIdentity, sliceBinding), true)
  } finally {
    releaseRun()
  }

  assert.deepEqual(await start, { status: "started" })
  await reserve
  assert.equal(Object.keys(fixture.baseStore.loadRequired().reservations).length, 1)
  assert.equal(hasMatchingSliceDiskQuotaUnboundedProof(fixture.paths.proofRoot, sliceIdentity, sliceBinding), false)
})

linuxTest("an abruptly exited lock owner releases its flock for a fresh coordinator", async (context) => {
  const fixture = await setup(context)
  const sliceIdentity = identity("lock-crash")
  const childScript = join(fixture.root, "crashed-lock-owner.mjs")
  const coordinatorUrl = new URL("./slice-disk-quota-coordinator.mjs", import.meta.url).href
  await writeFile(childScript, `
    import { createSliceDiskQuotaCoordinator } from ${JSON.stringify(coordinatorUrl)}
    import { writeSync } from "node:fs"
    const coordinator = createSliceDiskQuotaCoordinator({
      statePath: ${JSON.stringify(fixture.statePath)},
      coordinationRoot: ${JSON.stringify(fixture.paths.coordinationRoot)},
      proofRoot: ${JSON.stringify(fixture.paths.proofRoot)},
      lockWaitMs: 1_500,
    })
    await coordinator.withContainerLock(${JSON.stringify(sliceIdentity.containerName)}, async () => {
      writeSync(1, "lock-action-entered\\n")
      process.exit(0)
    })
  `, { mode: 0o600 })

  const child = spawn(process.execPath, [childScript], {
    cwd: "/",
    env: { PATH: "/usr/bin:/bin", LANG: "C" },
    stdio: ["ignore", "pipe", "ignore"],
  })
  const exited = once(child, "exit")
  context.after(async () => {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL")
      await exited
    }
  })
  let output = ""
  let signalEntered
  const entered = new Promise((resolve) => { signalEntered = resolve })
  child.stdout.on("data", (chunk) => {
    output += chunk.toString("utf8")
    if (output.includes("lock-action-entered\n")) signalEntered()
  })
  let deadline
  try {
    const didEnter = await Promise.race([
      entered.then(() => true),
      exited.then(() => false),
      new Promise((resolve) => { deadline = setTimeout(() => resolve(false), 4_000) }),
    ])
    assert.equal(didEnter, true)
    clearTimeout(deadline)
    const didExit = await Promise.race([
      exited.then(() => true),
      new Promise((resolve) => { deadline = setTimeout(() => resolve(false), 4_000) }),
    ])
    assert.equal(didExit, true)
  } finally {
    clearTimeout(deadline)
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL")
      await exited
    }
  }

  await fixture.coordinator.withContainerLock(sliceIdentity.containerName, (lock) => {
    fixture.coordinator.assertLockHeld(lock)
  })
})

linuxTest("failed bounded state write leaves the receipt revoked", async (context) => {
  const root = await mkdtemp(join(tmpdir(), "slice-disk-quota-failed-write-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const statePath = join(root, "state", "reservations.json")
  const baseStore = createFileSliceDiskQuotaStateStore(statePath)
  baseStore.save(emptyState())
  let failSave = false
  const stateStore = {
    load: () => baseStore.load(),
    loadRequired: () => baseStore.loadRequired(),
    save(state) {
      if (failSave) throw new Error("fixture durable write failure")
      baseStore.save(state)
    },
  }
  const paths = {
    statePath,
    coordinationRoot: join(root, "state", "coordination"),
    proofRoot: join(root, "state", "unbounded-proofs"),
  }
  const coordinator = createSliceDiskQuotaCoordinator({ stateStore, ...paths, lockWaitMs: 1_500 })
  const sliceIdentity = identity("writefail")
  const sliceBinding = binding(sliceIdentity.homeVolumeName)
  await coordinator.withContainerLock(sliceIdentity.containerName, (lock) => (
    coordinator.captureUnboundedProof(lock, sliceIdentity, sliceBinding)
  ))
  failSave = true

  const socketRoot = await mkdtemp(join(tmpdir(), "slice-disk-quota-failed-write-socket-"))
  const socketPath = join(socketRoot, "allocator.sock")
  const server = createServer({ allowHalfOpen: true }, (socket) => (
    handleSliceDiskQuotaConnection(socket, testAllocator(stateStore), { coordinator, requestTimeoutMs: 1_000 })
  ))
  server.listen(socketPath)
  await once(server, "listening")
  context.after(async () => {
    await new Promise((resolve) => server.close(resolve))
    await rm(socketRoot, { recursive: true, force: true })
  })
  await assert.rejects(
    requestSliceDiskQuota(reserveRequest(sliceIdentity), { socketPath, requestTimeoutMs: 1_500 }),
    /fixture durable write failure/,
  )
  await coordinator.withContainerLock(sliceIdentity.containerName, async (lock) => {
    assert.equal(await coordinator.resolveUnboundedProof(lock, sliceIdentity, sliceBinding), undefined)
  })
  assert.equal(await readFile(statePath, "utf8").then((text) => text.length > 0), true)
})
