import assert from "node:assert/strict"
import test from "node:test"
import {
  SLICE_DISK_QUOTA_PROJECT_ID_MIN,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  sliceDiskQuotaIdentityKey,
  validateSliceDiskQuotaIdentity,
  validateSliceDiskQuotaRequest,
  validateSliceDiskQuotaState,
} from "./slice-disk-quota-contract.mjs"

const identity = {
  ownerKernelId: "kernel-1",
  ownerMachineId: "machine-1",
  sliceId: "slice-1",
  containerName: "chariox-slice-one",
  homeVolumeName: "chariox-slice-one-home",
}

const request = {
  protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  operation: "reserve",
  identity,
  limits: { writableLayerBytes: 512 * 1024 * 1024, persistentHomeBytes: 2_048 * 1024 * 1024 },
}

test("versioned quota requests accept paired bounded limits and stable identities", () => {
  assert.equal(validateSliceDiskQuotaRequest(request), request)
  assert.equal(sliceDiskQuotaIdentityKey(identity), "kernel-1\0machine-1\0slice-1")
  assert.equal(validateSliceDiskQuotaIdentity(identity), identity)
})

test("quota requests reject unbound, malformed, forged, and replayed shapes", () => {
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, protocolVersion: 0 }), /version/)
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, extra: "path" }), /unsupported fields/)
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, identity: { ...identity, containerName: "arbitrary" } }), /identity/)
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, identity: { ...identity, homeVolumeName: "/tmp/foreign" } }), /identity/)
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, limits: { ...request.limits, writableLayerBytes: "512m" } }), /whole MiB/)
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, limits: { ...request.limits, persistentHomeBytes: 0 } }), /whole MiB/)
  assert.throws(() => validateSliceDiskQuotaRequest({ ...request, limits: { ...request.limits, persistentHomeBytes: 4_294_967_296 * 1024 * 1024 } }), /whole MiB/)
})

test("only fixed lifecycle operations can cross the quota allocator boundary", () => {
  for (const operation of ["apply_home", "apply_layer", "verify", "release"]) {
    assert.equal(validateSliceDiskQuotaRequest({ protocolVersion: 1, operation, identity }).operation, operation)
  }
  assert.equal(validateSliceDiskQuotaRequest({ protocolVersion: 1, operation: "ensure_before_start", containerName: identity.containerName }).operation, "ensure_before_start")
  assert.throws(() => validateSliceDiskQuotaRequest({ protocolVersion: 1, operation: "exec", command: "xfs_quota" }), /not allowed/)
  assert.throws(() => validateSliceDiskQuotaRequest({ protocolVersion: 1, operation: "apply_home", identity, path: "/tmp" }), /unsupported fields/)
})

test("durable reservation state rejects stale keys, duplicate IDs, and unsupported versions", () => {
  const key = sliceDiskQuotaIdentityKey(identity)
  const state = {
    schemaVersion: 1,
    nextProjectId: 1_073_741_826,
    reservations: {
      [key]: {
        identity,
        limits: request.limits,
        projectIds: { writableLayer: 1_073_741_824, persistentHome: 1_073_741_825 },
      },
    },
  }
  assert.equal(validateSliceDiskQuotaState(state), state)
  assert.throws(() => validateSliceDiskQuotaState({ ...state, schemaVersion: 2 }), /version/)
  assert.throws(() => validateSliceDiskQuotaState({ ...state, nextProjectId: 1_073_741_825 }), /replay/)
  assert.throws(() => validateSliceDiskQuotaState({ ...state, nextProjectId: 2_147_483_649 }), /allocator is invalid/)
  assert.throws(() => validateSliceDiskQuotaState({
    ...state,
    reservations: { [key]: { ...state.reservations[key], projectIds: { writableLayer: 1_073_741_824, persistentHome: 1_073_741_824 } } },
  }), /duplicated/)
  assert.throws(() => validateSliceDiskQuotaState({
    ...state,
    reservations: { [key]: { ...state.reservations[key], identity: { ...identity, sliceId: "another" } } },
  }), /stale/)
  const duplicateIdentity = { ...identity, sliceId: "slice-2" }
  const duplicateKey = sliceDiskQuotaIdentityKey(duplicateIdentity)
  assert.throws(() => validateSliceDiskQuotaState({
    ...state,
    nextProjectId: 1_073_741_828,
    reservations: {
      ...state.reservations,
      [duplicateKey]: {
        identity: duplicateIdentity,
        limits: request.limits,
        projectIds: { writableLayer: 1_073_741_826, persistentHome: 1_073_741_827 },
      },
    },
  }), /container name is duplicated/)
})

test("durable state validates 70,000 reservations without a spread-argument limit", () => {
  const count = 70_000
  const reservations = {}
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
      limits: request.limits,
      projectIds: { writableLayer: firstProjectId, persistentHome: firstProjectId + 1 },
    }
  }
  const state = {
    schemaVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    nextProjectId: SLICE_DISK_QUOTA_PROJECT_ID_MIN + count * 2,
    reservations,
  }

  assert.equal(validateSliceDiskQuotaState(state), state)
})

test("quota accepts only the owning container's legacy or protected generation home", () => {
  for (const homeVolumeName of [identity.homeVolumeName, `${identity.containerName}-home-g${"a".repeat(32)}`]) {
    assert.equal(validateSliceDiskQuotaRequest({protocolVersion: 1, operation: "apply_home",
      identity: {...identity, homeVolumeName}}).identity.homeVolumeName, homeVolumeName)
  }
  for (const homeVolumeName of [`chariox-slice-foreign-home-g${"a".repeat(32)}`,
    `${identity.containerName}-home-gshort`, `${identity.containerName}-home-g${"A".repeat(32)}`,
    `${identity.containerName}-home-g${"a".repeat(32)}/../other`]) {
    assert.throws(() => validateSliceDiskQuotaRequest({protocolVersion: 1, operation: "apply_home",
      identity: {...identity, homeVolumeName}}), /identity/)
  }
})

test("retained home project IDs remain unique, bounded and tracked by durable state", () => {
  const min = SLICE_DISK_QUOTA_PROJECT_ID_MIN
  const state = {schemaVersion: 1, nextProjectId: min + 3, reservations: {
    [sliceDiskQuotaIdentityKey(identity)]: {identity, limits: request.limits,
      projectIds: {writableLayer: min, persistentHome: min + 2}, retainedHomeProjectIds: [min + 1]},
  }}
  assert.equal(validateSliceDiskQuotaState(state), state)
  for (const ids of [[min], [min + 1, min + 1], [0], "invalid"]) {
    const invalid = structuredClone(state)
    invalid.reservations[sliceDiskQuotaIdentityKey(identity)].retainedHomeProjectIds = ids
    assert.throws(() => validateSliceDiskQuotaState(invalid), /project ID/)
  }
})
