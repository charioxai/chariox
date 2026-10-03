import {
  SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES,
  SLICE_DISK_QUOTA_PROJECT_ID_MAX,
  SLICE_DISK_QUOTA_PROJECT_ID_MIN,
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  sliceDiskQuotaIdentityKey,
  validateSliceDiskQuotaRequest,
  validateSliceDiskQuotaState,
} from "./slice-disk-quota-contract.mjs"

function fail(message) {
  throw new Error(message)
}

function defaultState() {
  return {
    schemaVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    nextProjectId: SLICE_DISK_QUOTA_PROJECT_ID_MIN,
    reservations: {},
  }
}

function normalizeState(state) {
  return validateSliceDiskQuotaState(state)
}

function evidenceClass(supported, persistent, effectiveLimitBytes, usedBytes) {
  return {
    backendSupportsHardQuota: supported,
    isPersistent: persistent,
    effectiveLimitBytes,
    usedBytes,
  }
}

function bytesWithin(usedBytes, limitBytes, label) {
  if (!Number.isSafeInteger(usedBytes) || usedBytes < 0) fail(`${label} quota usage could not be read back`)
  if (usedBytes > limitBytes) fail(`existing ${label} data exceeds its configured hard quota`)
}

function exactQuota(result, limitBytes, label) {
  if (!result?.treeVerified || result.effectiveLimitBytes !== limitBytes) {
    fail(`${label} project quota tree or exact hard limit could not be verified`)
  }
  bytesWithin(result.usedBytes, limitBytes, label)
  return result
}

function evidenceFromResults(layer, home) {
  return {
    writableLayer: evidenceClass(true, false, layer.effectiveLimitBytes, layer.usedBytes),
    persistentHome: evidenceClass(true, true, home.effectiveLimitBytes, home.usedBytes),
  }
}

export function createSliceDiskQuotaAllocator({ backend, stateStore }) {
  if (!backend || !stateStore) throw new TypeError("disk quota allocator requires backend and durable state store")

  function loadState() {
    return normalizeState(stateStore.load())
  }

  function persist(state) {
    stateStore.save(normalizeState(state))
  }

  function reservationFor(state, identity) {
    return state.reservations[sliceDiskQuotaIdentityKey(identity)]
  }

  function requireReservation(state, identity) {
    const reservation = reservationFor(state, identity)
    if (!reservation) fail("slice has no durable disk quota reservation")
    return reservation
  }

  function requireSupportedProbe() {
    const probe = backend.probe()
    if (!probe?.supported) fail(probe?.reason ?? "managed disk quota backend is unsupported")
    if (!Number.isSafeInteger(probe.totalBytes) || probe.totalBytes <= 0) fail("quota backend capacity could not be verified")
    if (!Number.isSafeInteger(probe.availableBytes) || probe.availableBytes < 0) fail("quota backend free space could not be verified")
    if (!Number.isSafeInteger(probe.hostTotalBytes) || probe.hostTotalBytes <= 0) fail("host recovery capacity could not be verified")
    if (!Number.isSafeInteger(probe.hostAvailableBytes) || probe.hostAvailableBytes < 0) fail("host recovery free space could not be verified")
    return probe
  }

  function reserveCapacity(state, identity, limits, probe) {
    const key = identity === undefined ? undefined : sliceDiskQuotaIdentityKey(identity)
    let outstandingBytes = 0
    for (const [reservationKey, record] of Object.entries(state.reservations)) {
      const proposed = reservationKey === key ? limits : record.limits
      for (const [storageClass, cap, projectId] of [
        ["writable-layer", proposed.writableLayerBytes, record.projectIds.writableLayer],
        ["persistent-home", proposed.persistentHomeBytes, record.projectIds.persistentHome],
      ]) {
        const usedBytes = backend.projectUsageBytes(projectId)
        if (!Number.isSafeInteger(usedBytes) || usedBytes < 0) {
          fail(`${storageClass} reservation usage could not be read back`)
        }
        if (usedBytes > cap) fail(`existing ${storageClass} data exceeds its configured hard quota`)
        outstandingBytes += cap - usedBytes
      }
    }
    if (key === undefined || !state.reservations[key]) {
      outstandingBytes += limits.writableLayerBytes + limits.persistentHomeBytes
    }
    if (!Number.isSafeInteger(outstandingBytes)) fail("disk quota reservation exceeds the safe aggregate range")
    const quotaReserveBytes = Math.max(
      SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES,
      Math.ceil(probe.totalBytes / 10),
    )
    if (probe.availableBytes < outstandingBytes + quotaReserveBytes) {
      fail("disk quota reservation would consume the managed Docker XFS recovery reserve")
    }
    const hostReserveBytes = Math.max(
      SLICE_DISK_QUOTA_HOST_RESERVE_MIN_BYTES,
      Math.ceil(probe.hostTotalBytes / 10),
    )
    if (probe.hostAvailableBytes < hostReserveBytes) {
      fail("managed host filesystem is below its recovery reserve")
    }
  }

  function allocateProjectIds(state) {
    const occupied = new Set(backend.projectIdsInUse())
    for (const record of Object.values(state.reservations)) {
      occupied.add(record.projectIds.writableLayer)
      occupied.add(record.projectIds.persistentHome)
    }
    let candidate = state.nextProjectId
    const next = () => {
      while (candidate <= SLICE_DISK_QUOTA_PROJECT_ID_MAX && occupied.has(candidate)) candidate += 1
      if (candidate > SLICE_DISK_QUOTA_PROJECT_ID_MAX) fail("managed disk quota project ID range is exhausted")
      const allocated = candidate
      occupied.add(allocated)
      candidate += 1
      return allocated
    }
    const projectIds = { writableLayer: next(), persistentHome: next() }
    state.nextProjectId = candidate
    return projectIds
  }

  function requireIdentityBinding(record, target, label) {
    if (!target || target.driver !== "local" || target.persistent !== true) {
      fail(`${label} is not a persistent Docker local volume`)
    }
    const labels = target.labels ?? {}
    for (const [labelName, value] of Object.entries(expectedIdentityLabels(record.identity))) {
      if (labels[labelName] !== value) fail(`${label} Docker identity labels do not match its reservation`)
    }
  }

  function expectedIdentityLabels(identity) {
    return {
      "io.chariox.slice.id": identity.sliceId,
      "io.chariox.slice.owner-kernel-id": identity.ownerKernelId,
      "io.chariox.slice.owner-machine-id": identity.ownerMachineId,
    }
  }

  function applyHome(record) {
    requireSupportedProbe()
    const target = backend.inspectHome(record.identity)
    requireIdentityBinding(record, target, "persistent home")
    const result = backend.applyHardQuota({
      storageClass: "persistentHome",
      path: target.path,
      projectId: record.projectIds.persistentHome,
      limitBytes: record.limits.persistentHomeBytes,
      state: backend.inspectContainer(record.identity)?.state ?? "absent",
    })
    return exactQuota(result, record.limits.persistentHomeBytes, "persistent-home")
  }

  function applyLayer(record) {
    requireSupportedProbe()
    const target = backend.inspectLayer(record.identity)
    if (!target || target.driver !== "overlay2") fail("Docker does not expose a trusted overlay2 writable-layer UpperDir")
    for (const [labelName, value] of Object.entries(expectedIdentityLabels(record.identity))) {
      if (target.labels?.[labelName] !== value) fail("writable-layer Docker identity labels do not match its reservation")
    }
    const result = backend.applyHardQuota({
      storageClass: "writableLayer",
      paths: target.paths,
      projectId: record.projectIds.writableLayer,
      limitBytes: record.limits.writableLayerBytes,
      state: target.state,
    })
    return exactQuota(result, record.limits.writableLayerBytes, "writable-layer")
  }

  function bindHome(record, identity, state) {
    if (record.identity.containerName !== identity.containerName) fail("quota reservation identity changed")
    if (record.identity.homeVolumeName === identity.homeVolumeName) return
    const container = backend.inspectContainer(record.identity)
    if (container && !["created", "exited", "dead"].includes(container.state)) {
      fail("stop the slice before changing its disk quota home generation")
    }
    // Reserve a full new home cap before restoring. Previous generations stay
    // capped independently and their actual bytes remain in filesystem usage.
    reserveCapacity(state, undefined, {writableLayerBytes: 0, persistentHomeBytes: record.limits.persistentHomeBytes}, requireSupportedProbe())
    const next = {...record, identity: {...identity}, projectIds: {...record.projectIds,
      persistentHome: allocateProjectIds(state).persistentHome}}
    applyHome(next) // Verify the new volume labels/tree/cap before publication.
    record.identity = next.identity
    record.projectIds = next.projectIds
    persist(state)
  }

  function verifyRecord(record) {
    requireSupportedProbe()
    const homeTarget = backend.inspectHome(record.identity)
    requireIdentityBinding(record, homeTarget, "persistent home")
    const home = backend.verifyHardQuota({
      storageClass: "persistentHome",
      path: homeTarget.path,
      projectId: record.projectIds.persistentHome,
      limitBytes: record.limits.persistentHomeBytes,
    })
    const layerTarget = backend.inspectLayer(record.identity)
    if (!layerTarget || layerTarget.driver !== "overlay2") fail("Docker does not expose a trusted overlay2 writable-layer UpperDir")
    for (const [labelName, value] of Object.entries(expectedIdentityLabels(record.identity))) {
      if (layerTarget.labels?.[labelName] !== value) fail("writable-layer Docker identity labels do not match its reservation")
    }
    const layer = backend.verifyHardQuota({
      storageClass: "writableLayer",
      paths: layerTarget.paths,
      projectId: record.projectIds.writableLayer,
      limitBytes: record.limits.writableLayerBytes,
    })
    exactQuota(home, record.limits.persistentHomeBytes, "persistent-home")
    exactQuota(layer, record.limits.writableLayerBytes, "writable-layer")
    return evidenceFromResults(layer, home)
  }

  function handle(request) {
    validateSliceDiskQuotaRequest(request)
    const state = loadState()
    if (request.operation === "probe") return backend.probe()

    if (request.operation === "reserve") {
      const key = sliceDiskQuotaIdentityKey(request.identity)
      const existing = state.reservations[key]
      const probe = requireSupportedProbe()
      if (existing) {
        bindHome(existing, request.identity, state)
        if (
          existing.limits.writableLayerBytes !== request.limits.writableLayerBytes ||
          existing.limits.persistentHomeBytes !== request.limits.persistentHomeBytes
        ) {
          const container = backend.inspectContainer(request.identity)
          if (container && !["created", "exited", "dead"].includes(container.state)) {
            fail("stop the slice before changing its disk quota limits")
          }
          reserveCapacity(state, request.identity, request.limits, probe)
          existing.limits = { ...request.limits }
          persist(state)
        }
        return { reserved: true, projectIds: { ...existing.projectIds } }
      }
      for (const record of Object.values(state.reservations)) {
        if (record.identity.containerName === request.identity.containerName) {
          fail("another slice already owns this managed Docker container name")
        }
      }
      reserveCapacity(state, undefined, request.limits, probe)
      const record = {
        identity: { ...request.identity },
        limits: { ...request.limits },
        projectIds: allocateProjectIds(state),
      }
      state.reservations[key] = record
      persist(state)
      return { reserved: true, projectIds: { ...record.projectIds } }
    }

    if (request.operation === "ensure_before_start") {
      const record = Object.values(state.reservations).find(
        (candidate) => candidate.identity.containerName === request.containerName,
      )
      if (!record) return { bounded: false }
      const container = backend.inspectContainer(record.identity)
      if (container?.homeVolumeName) {
        const identity = {...record.identity, homeVolumeName: container.homeVolumeName}
        validateSliceDiskQuotaRequest({protocolVersion: 1, operation: "apply_home", identity})
        bindHome(record, identity, state)
      }
      applyHome(record)
      applyLayer(record)
      return { bounded: true, evidence: verifyRecord(record) }
    }

    const key = sliceDiskQuotaIdentityKey(request.identity)
    if (request.operation === "status") {
      const record = state.reservations[key]
      if (!record) return { bounded: false }
      return { bounded: true, limits: { ...record.limits }, projectIds: { ...record.projectIds } }
    }
    if (request.operation === "release") {
      const record = state.reservations[key]
      if (!record) return { released: true }
      if (!backend.confirmContainerAndVolumeRemoved(record.identity)) {
        fail("disk quota reservation is retained until Docker container and volume removal are verified")
      }
      for (const id of Object.values(record.projectIds)) {
        backend.clearProjectQuota(id)
      }
      delete state.reservations[key]
      persist(state)
      return { released: true }
    }

    const record = requireReservation(state, request.identity)
    if (request.operation === "apply_home") {
      bindHome(record, request.identity, state)
      return { result: applyHome(record) }
    }
    if (JSON.stringify(record.identity) !== JSON.stringify(request.identity)) fail("quota reservation identity changed")
    if (request.operation === "apply_layer") return { result: applyLayer(record) }
    if (request.operation === "verify") return { evidence: verifyRecord(record) }
    fail("quota operation is not implemented")
  }

  return { handle }
}
