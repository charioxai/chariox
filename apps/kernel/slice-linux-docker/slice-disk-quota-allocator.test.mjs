import assert from "node:assert/strict"
import { chmodSync, mkdtempSync, rmSync, statSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import {
  SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES,
  SLICE_DISK_QUOTA_PROJECT_ID_MIN,
  sliceDiskQuotaIdentityKey,
} from "./slice-disk-quota-contract.mjs"
import {
  createSliceDiskQuotaAllocator,
} from "./slice-disk-quota-allocator.mjs"
import { createFileSliceDiskQuotaStateStore } from "./slice-disk-quota-state-store.mjs"

const identity = {
  ownerKernelId: "kernel-1",
  ownerMachineId: "machine-1",
  sliceId: "slice-1",
  containerName: "chariox-slice-one",
  homeVolumeName: "chariox-slice-one-home",
}
const limits = { writableLayerBytes: 512 * 1024 * 1024, persistentHomeBytes: 2_048 * 1024 * 1024 }

function fixture(overrides = {}) {
  let state = structuredClone(overrides.initialState ?? { schemaVersion: 1, nextProjectId: 1_073_741_824, reservations: {} })
  let availableBytes = 48 * 1024 ** 3
  const calls = []
  const actual = { layerUsed: 48 * 1024 * 1024, homeUsed: 256 * 1024 * 1024 }
  const backend = {
    probe: () => ({
      supported: true,
      totalBytes: 64 * 1024 ** 3,
      availableBytes,
      hostTotalBytes: 128 * 1024 ** 3,
      hostAvailableBytes: 32 * 1024 ** 3,
    }),
    projectIdsInUse: () => [],
    projectUsageBytes: id => id === 1_073_741_824 ? actual.layerUsed : actual.homeUsed,
    inspectHome: got => ({ driver: "local", persistent: true, labels: identityLabels(got), path: "/data/volumes/chariox-slice-one-home/_data" }),
    inspectLayer: got => ({
      driver: "overlay2",
      labels: identityLabels(got),
      paths: [
        "/data/overlay2/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/diff",
        "/data/overlay2/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/work",
      ],
      state: "exited",
    }),
    inspectContainer: () => ({ state: "exited" }),
    applyHardQuota: ({ storageClass, limitBytes, projectId, path, paths, state: dockerState }) => {
      calls.push(["apply", storageClass, path, paths, projectId, limitBytes, dockerState])
      return {
        treeVerified: true,
        effectiveLimitBytes: limitBytes,
        usedBytes: storageClass === "writableLayer" ? actual.layerUsed : actual.homeUsed,
      }
    },
    verifyHardQuota: ({ storageClass, limitBytes }) => ({
      treeVerified: true,
      effectiveLimitBytes: limitBytes,
      usedBytes: storageClass === "writableLayer" ? actual.layerUsed : actual.homeUsed,
    }),
    clearProjectQuota: id => calls.push(["clear", id]),
    confirmContainerAndVolumeRemoved: () => overrides.removed ?? true,
    ...overrides.backend,
  }
  const stateStore = {
    load: () => structuredClone(state),
    save: value => { state = structuredClone(value) },
  }
  return {
    allocator: createSliceDiskQuotaAllocator({ backend, stateStore }),
    calls,
    actual,
    getState: () => state,
    backend,
    stateStore,
    setAvailable: value => { availableBytes = value },
  }
}

function identityLabels(value) {
  return {
    "io.chariox.slice.id": value.sliceId,
    "io.chariox.slice.owner-kernel-id": value.ownerKernelId,
    "io.chariox.slice.owner-machine-id": value.ownerMachineId,
  }
}

function reserve(allocator, capLimits = limits) {
  return allocator.handle({ protocolVersion: 1, operation: "reserve", identity, limits: capLimits })
}

function stateWithReservationCount(count) {
  const reservations = {}
  let targetIdentity
  let targetProjectIds
  for (let index = 0; index < count; index += 1) {
    const containerName = `chariox-slice-${index}`
    const reservationIdentity = {
      ownerKernelId: identity.ownerKernelId,
      ownerMachineId: identity.ownerMachineId,
      sliceId: `slice-${index}`,
      containerName,
      homeVolumeName: `${containerName}-home`,
    }
    const firstProjectId = SLICE_DISK_QUOTA_PROJECT_ID_MIN + index * 2
    reservations[sliceDiskQuotaIdentityKey(reservationIdentity)] = {
      identity: reservationIdentity,
      limits,
      projectIds: { writableLayer: firstProjectId, persistentHome: firstProjectId + 1 },
    }
    if (index === count - 1) {
      targetIdentity = reservationIdentity
      targetProjectIds = { writableLayer: firstProjectId, persistentHome: firstProjectId + 1 }
    }
  }
  return {
    identity: targetIdentity,
    projectIds: targetProjectIds,
    state: {
      schemaVersion: 1,
      nextProjectId: SLICE_DISK_QUOTA_PROJECT_ID_MIN + count * 2,
      reservations,
    },
  }
}

test("reserve is durable, idempotent, and assigns independent IDs to both storage classes", () => {
  const { allocator, getState } = fixture()
  const first = reserve(allocator)
  const second = reserve(allocator)
  assert.deepEqual(first, second)
  assert.equal(first.projectIds.writableLayer, 1_073_741_824)
  assert.equal(first.projectIds.persistentHome, 1_073_741_825)
  assert.deepEqual(getState().reservations["kernel-1\0machine-1\0slice-1"].limits, limits)
})

test("filesystem reservations survive allocator recreation and unsafe state mode is rejected", () => {
  const root = mkdtempSync(join(tmpdir(), "chariox-slice-quota-state-"))
  const statePath = join(root, "state", "reservations.json")
  try {
    const stateStore = createFileSliceDiskQuotaStateStore(statePath)
    const { backend } = fixture()
    reserve(createSliceDiskQuotaAllocator({ backend, stateStore }))
    assert.equal(statSync(statePath).mode & 0o777, 0o600)
    assert.equal(statSync(join(root, "state")).mode & 0o777, 0o700)

    const restarted = createSliceDiskQuotaAllocator({ backend, stateStore })
    assert.deepEqual(restarted.handle({ protocolVersion: 1, operation: "status", identity }), {
      bounded: true,
      limits,
      projectIds: { writableLayer: 1_073_741_824, persistentHome: 1_073_741_825 },
    })

    chmodSync(statePath, 0o644)
    assert.throws(() => stateStore.load(), /state file is unsafe/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test("restarted allocator can release from 70,000 valid durable reservations", () => {
  const { identity: releaseIdentity, projectIds, state } = stateWithReservationCount(70_000)
  const seeded = fixture({ initialState: state })
  const restarted = createSliceDiskQuotaAllocator({ backend: seeded.backend, stateStore: seeded.stateStore })

  assert.deepEqual(restarted.handle({ protocolVersion: 1, operation: "release", identity: releaseIdentity }), { released: true })
  assert.equal(Object.keys(seeded.getState().reservations).length, 69_999)
  assert.equal(seeded.getState().reservations[sliceDiskQuotaIdentityKey(releaseIdentity)], undefined)
  assert.deepEqual(seeded.calls, [["clear", projectIds.writableLayer], ["clear", projectIds.persistentHome]])
})

test("reserve rejects unsupported backends and preserves the host recovery reserve", () => {
  const unsupported = fixture({ backend: { probe: () => ({ supported: false, reason: "ext4 is unsupported" }) } })
  assert.throws(() => reserve(unsupported.allocator), /ext4 is unsupported/)
  assert.deepEqual(unsupported.getState().reservations, {})

  const lowSpace = fixture({ backend: { probe: () => ({ supported: true, totalBytes: 64 * 1024 ** 3, availableBytes: SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES + 1, hostTotalBytes: 64 * 1024 ** 3, hostAvailableBytes: 32 * 1024 ** 3 }) } })
  assert.throws(() => reserve(lowSpace.allocator), /XFS recovery reserve/)
  assert.deepEqual(lowSpace.getState().reservations, {})

  const lowHost = fixture({ backend: { probe: () => ({ supported: true, totalBytes: 64 * 1024 ** 3, availableBytes: 48 * 1024 ** 3, hostTotalBytes: 64 * 1024 ** 3, hostAvailableBytes: SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES - 1 }) } })
  assert.throws(() => reserve(lowHost.allocator), /host filesystem.*recovery reserve/)
  assert.deepEqual(lowHost.getState().reservations, {})
})

test("reserve updates account for current use once and reject missing usage readback or live cap changes", () => {
  const updated = fixture()
  reserve(updated.allocator)
  const nextLimits = { writableLayerBytes: 1_024 * 1024 * 1024, persistentHomeBytes: 3_072 * 1024 * 1024 }
  const usedBytes = updated.actual.layerUsed + updated.actual.homeUsed
  updated.setAvailable(
    nextLimits.writableLayerBytes + nextLimits.persistentHomeBytes - usedBytes +
      Math.max(SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES, Math.ceil((64 * 1024 ** 3) / 10)),
  )
  assert.equal(reserve(updated.allocator, nextLimits).reserved, true)

  const unreadable = fixture()
  reserve(unreadable.allocator)
  unreadable.backend.projectUsageBytes = () => undefined
  assert.throws(() => reserve(unreadable.allocator, nextLimits), /usage could not be read back/)

  const running = fixture({ backend: { inspectContainer: () => ({ state: "running" }) } })
  reserve(running.allocator)
  assert.throws(() => reserve(running.allocator, nextLimits), /stop the slice before changing/)

  const paused = fixture({ backend: { inspectContainer: () => ({ state: "paused" }) } })
  reserve(paused.allocator)
  assert.throws(() => reserve(paused.allocator, nextLimits), /stop the slice before changing/)
})

test("home and writable-layer quotas are separately applied and both produce admission evidence", () => {
  const { allocator, calls } = fixture()
  reserve(allocator)
  allocator.handle({ protocolVersion: 1, operation: "apply_home", identity })
  allocator.handle({ protocolVersion: 1, operation: "apply_layer", identity })
  const verified = allocator.handle({ protocolVersion: 1, operation: "verify", identity }).evidence
  assert.equal(calls[0][1], "persistentHome")
  assert.equal(calls[1][1], "writableLayer")
  assert.deepEqual(calls[1][3], [
    "/data/overlay2/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/diff",
    "/data/overlay2/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/work",
  ])
  assert.equal(calls[1][4], 1_073_741_824)
  assert.equal(verified.writableLayer.backendSupportsHardQuota, true)
  assert.equal(verified.writableLayer.effectiveLimitBytes, limits.writableLayerBytes)
  assert.equal(verified.persistentHome.isPersistent, true)
  assert.equal(verified.persistentHome.effectiveLimitBytes, limits.persistentHomeBytes)
})

test("start and unpause guards require both quotas before returning", () => {
  const { allocator } = fixture()
  reserve(allocator)
  const ensured = allocator.handle({ protocolVersion: 1, operation: "ensure_before_start", containerName: identity.containerName })
  assert.equal(ensured.bounded, true)
  assert.equal(ensured.evidence.writableLayer.effectiveLimitBytes, limits.writableLayerBytes)
  assert.deepEqual(allocator.handle({ protocolVersion: 1, operation: "ensure_before_start", containerName: "chariox-slice-other" }), { bounded: false })
})

test("oversized existing home or layer data fails closed after quota readback", () => {
  const home = fixture()
  home.actual.homeUsed = limits.persistentHomeBytes + 1
  reserve(home.allocator)
  assert.throws(() => home.allocator.handle({ protocolVersion: 1, operation: "apply_home", identity }), /existing persistent-home data exceeds/)

  const layer = fixture()
  layer.actual.layerUsed = limits.writableLayerBytes + 1
  reserve(layer.allocator)
  assert.throws(() => layer.allocator.handle({ protocolVersion: 1, operation: "apply_layer", identity }), /existing writable-layer data exceeds/)
})

test("missing overlay2 mapping, foreign volume labels, and failed hard-limit readback are rejected", () => {
  const noMapping = fixture({ backend: { inspectLayer: () => ({ driver: "overlayfs", paths: ["/data/containerd/snapshots/1/fs"], state: "exited" }) } })
  reserve(noMapping.allocator)
  assert.throws(() => noMapping.allocator.handle({ protocolVersion: 1, operation: "apply_layer", identity }), /overlay2 writable-layer UpperDir/)

  const foreignHome = fixture({ backend: { inspectHome: () => ({ driver: "local", persistent: true, labels: {}, path: "/data/volumes/chariox-slice-one-home/_data" }) } })
  reserve(foreignHome.allocator)
  assert.throws(() => foreignHome.allocator.handle({ protocolVersion: 1, operation: "apply_home", identity }), /labels do not match/)

  const unreadable = fixture({ backend: { applyHardQuota: () => ({ treeVerified: true, effectiveLimitBytes: undefined, usedBytes: 0 }) } })
  reserve(unreadable.allocator)
  assert.throws(() => unreadable.allocator.handle({ protocolVersion: 1, operation: "apply_home", identity }), /could not be verified/)
})

test("release retains a reservation until both Docker objects and quota usage are verified gone", () => {
  const retained = fixture({ removed: false })
  reserve(retained.allocator)
  assert.throws(() => retained.allocator.handle({ protocolVersion: 1, operation: "release", identity }), /retained until Docker container and volume removal/)
  assert.equal(Object.keys(retained.getState().reservations).length, 1)

  const removed = fixture()
  reserve(removed.allocator)
  assert.deepEqual(removed.allocator.handle({ protocolVersion: 1, operation: "release", identity }), { released: true })
  assert.equal(Object.keys(removed.getState().reservations).length, 0)
  assert.equal(removed.calls.filter(([kind]) => kind === "clear").length, 2)
})

test("protected restore applies quota to the new home and keeps retained homes in the same project", () => {
  const seen = []
  const f = fixture({backend: {inspectHome: got => {
    seen.push(got.homeVolumeName)
    return {driver: "local", persistent: true, labels: identityLabels(got), path: `/data/volumes/${got.homeVolumeName}/_data`}
  }}})
  const first = reserve(f.allocator)
  const restored = {...identity, homeVolumeName: `${identity.containerName}-home-g${"a".repeat(32)}`}
  f.allocator.handle({protocolVersion: 1, operation: "apply_home", identity: restored})
  assert.equal(seen.at(-1), restored.homeVolumeName)
  const record = f.getState().reservations[sliceDiskQuotaIdentityKey(identity)]
  assert.deepEqual(record.projectIds, first.projectIds)
  assert.equal(record.identity.homeVolumeName, restored.homeVolumeName)
  f.allocator.handle({protocolVersion: 1, operation: "verify", identity: restored})
  assert.equal(seen.at(-1), restored.homeVolumeName)
  assert.throws(() => f.allocator.handle({protocolVersion: 1, operation: "verify", identity}), /identity changed/)
  f.backend.inspectContainer = () => ({state: "running"})
  assert.throws(() => f.allocator.handle({protocolVersion: 1, operation: "apply_home", identity}), /stop the slice/)
  assert.equal(record.identity.homeVolumeName, restored.homeVolumeName)
})
