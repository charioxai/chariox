import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"
import { createSliceDiskQuotaAllocator } from "./slice-disk-quota-allocator.mjs"
import {
  readSliceDiskQuotaMarkerInspection,
  runWithSliceDiskQuotaAdmission,
} from "./slice-disk-quota-admission.mjs"

const verifiedResult = {
  bounded: true,
  evidence: {
    writableLayer: {
      backendSupportsHardQuota: true,
      isPersistent: false,
      effectiveLimitBytes: 1024,
      usedBytes: 256,
    },
    persistentHome: {
      backendSupportsHardQuota: true,
      isPersistent: true,
      effectiveLimitBytes: 2048,
      usedBytes: 512,
    },
  },
}

test("quota marker reads accept only parsed labels or an exact Docker not-found result", () => {
  assert.equal(readSliceDiskQuotaMarkerInspection({ status: 0, stdout: "{}" }, "container", "chariox-slice-test"), false)
  assert.equal(readSliceDiskQuotaMarkerInspection({ status: 0, stdout: "null" }, "volume", "chariox-slice-test-home"), false)
  assert.equal(readSliceDiskQuotaMarkerInspection({
    status: 0,
    stdout: JSON.stringify({ "io.chariox.slice.disk-quota": "xfs-project-v1" }),
  }, "container", "chariox-slice-test"), true)
  assert.equal(readSliceDiskQuotaMarkerInspection({
    status: 1,
    stderr: "Error: No such object: chariox-slice-test",
  }, "container", "chariox-slice-test"), false)
  assert.equal(readSliceDiskQuotaMarkerInspection({
    status: 1,
    stderr: "Error response from daemon: get chariox-slice-test-home: no such volume",
  }, "volume", "chariox-slice-test-home"), false)
})

test("unreadable, malformed, or mismatched quota marker reads fail closed", () => {
  const invalidReads = [
    [{ status: null, signal: "SIGTERM" }, "container", "chariox-slice-test"],
    [{ status: 1, stderr: "permission denied" }, "container", "chariox-slice-test"],
    [{ status: 1, stderr: "Error: No such object: chariox-slice-other" }, "container", "chariox-slice-test"],
    [{ status: 0, stdout: "not-json" }, "volume", "chariox-slice-test-home"],
    [{ status: 0, stdout: JSON.stringify({ "io.chariox.slice.disk-quota": "future-version" }) }, "volume", "chariox-slice-test-home"],
  ]
  for (const [result, kind, name] of invalidReads) {
    assert.throws(() => readSliceDiskQuotaMarkerInspection(result, kind, name))
  }
})

test("the broker validates container and volume inspections before deriving marker absence", async () => {
  const source = await readFile(new URL("./managed-docker-broker.mjs", import.meta.url), "utf8")
  const markerReader = source.slice(
    source.indexOf("function diskQuotaMarkerPresent("),
    source.indexOf("function requireExactContainerMounts(", source.indexOf("function diskQuotaMarkerPresent(")),
  )
  assert.match(source, /readSliceDiskQuotaMarkerInspection,[\s\S]*runWithSliceDiskQuotaAdmission/)
  assert.match(markerReader, /readSliceDiskQuotaMarkerInspection\(inspectedContainer, "container", container\)/)
  assert.match(markerReader, /readSliceDiskQuotaMarkerInspection\(inspectedVolume, "volume", volume\)/)
})

function admission({ containerName = "chariox-slice-test", quotaMarkerPresent, result, requestQuota, run }) {
  return runWithSliceDiskQuotaAdmission({
    containerName,
    quotaMarkerPresent,
    requestQuota: requestQuota ?? (async () => result),
    run,
  })
}

test("a quota-marked container cannot start without its matching durable reservation", async () => {
  let mutationCount = 0
  let receivedRequest
  await assert.rejects(admission({
    quotaMarkerPresent: true,
    result: { bounded: false },
    requestQuota: async (request) => {
      receivedRequest = request
      return { bounded: false }
    },
    run: () => { mutationCount += 1 },
  }), /quota marker but no matching durable reservation/)
  assert.deepEqual(receivedRequest, {
    protocolVersion: 1,
    operation: "ensure_before_start",
    containerName: "chariox-slice-test",
  })
  assert.equal(mutationCount, 0)
})

test("a quota-marked container starts only after exact hard-quota evidence", async () => {
  let mutationResult
  const result = await admission({
    quotaMarkerPresent: true,
    result: verifiedResult,
    run: (admissionResult) => {
      mutationResult = admissionResult
      return "started"
    },
  })
  assert.equal(result, "started")
  assert.deepEqual(mutationResult, verifiedResult)
})

test("an unmarked unbounded container keeps the existing start behavior", async () => {
  let mutationCount = 0
  const result = await admission({
    quotaMarkerPresent: false,
    result: { bounded: false },
    run: () => {
      mutationCount += 1
      return "started"
    },
  })
  assert.equal(result, "started")
  assert.equal(mutationCount, 1)
})

test("missing or malformed allocator results fail before the container mutation", async () => {
  const invalidResults = [
    undefined,
    null,
    {},
    { bounded: true },
    { bounded: false, evidence: {} },
    { bounded: true, evidence: { ...verifiedResult.evidence, extra: true } },
    {
      bounded: true,
      evidence: {
        ...verifiedResult.evidence,
        writableLayer: { ...verifiedResult.evidence.writableLayer, isPersistent: true },
      },
    },
  ]
  for (const result of invalidResults) {
    let mutationCount = 0
    await assert.rejects(admission({
      quotaMarkerPresent: false,
      result,
      run: () => { mutationCount += 1 },
    }))
    assert.equal(mutationCount, 0)
  }
})

test("unavailable quota authority and a wrong-container response never fall back to Docker", async () => {
  let mutationCount = 0
  await assert.rejects(admission({
    quotaMarkerPresent: true,
    requestQuota: async () => {
      const error = new Error("allocator unavailable")
      error.code = "ECONNREFUSED"
      throw error
    },
    run: () => { mutationCount += 1 },
  }), /allocator unavailable/)

  let requestedContainer
  await assert.rejects(admission({
    containerName: "chariox-slice-wrong",
    quotaMarkerPresent: true,
    requestQuota: async (request) => {
      requestedContainer = request.containerName
      return { bounded: false }
    },
    run: () => { mutationCount += 1 },
  }), /quota marker but no matching durable reservation/)
  assert.equal(requestedContainer, "chariox-slice-wrong")
  assert.equal(mutationCount, 0)
})

test("an absent marker does not authorize fallback on missing, refused, or arbitrary allocator errors", async () => {
  for (const code of ["ENOENT", "ECONNREFUSED", "EIO"]) {
    let mutationCount = 0
    const unavailable = new Error("quota authority is unavailable")
    unavailable.code = code
    await assert.rejects(admission({
      quotaMarkerPresent: false,
      requestQuota: async () => { throw unavailable },
      run: () => { mutationCount += 1 },
    }), (error) => error === unavailable)
    assert.equal(mutationCount, 0)
  }
})

test("marker absence after broker restart cannot hide a persisted bounded reservation", async () => {
  let persisted = {
    schemaVersion: 1,
    nextProjectId: 1_073_741_824,
    reservations: {},
  }
  const stateStore = {
    load: () => structuredClone(persisted),
    save: (state) => { persisted = structuredClone(state) },
  }
  const backend = {
    probe: () => ({
      supported: true,
      totalBytes: 8 * 1024 ** 3,
      availableBytes: 8 * 1024 ** 3,
      hostTotalBytes: 8 * 1024 ** 3,
      hostAvailableBytes: 8 * 1024 ** 3,
    }),
    projectIdsInUse: () => [],
  }
  const identity = {
    ownerKernelId: "kernel-legacy",
    ownerMachineId: "machine-legacy",
    sliceId: "slice-legacy",
    containerName: "chariox-slice-legacy",
    homeVolumeName: "chariox-slice-legacy-home",
  }
  const limits = {
    writableLayerBytes: 1024 * 1024,
    persistentHomeBytes: 1024 * 1024,
  }
  createSliceDiskQuotaAllocator({ backend, stateStore }).handle({
    protocolVersion: 1,
    operation: "reserve",
    identity,
    limits,
  })

  // Reservation is durable before provisioning creates creation-time labels;
  // existing Docker objects also retain their original labels on quota upgrade.
  const restartedAllocator = createSliceDiskQuotaAllocator({ backend, stateStore })
  assert.equal(restartedAllocator.handle({ protocolVersion: 1, operation: "status", identity }).bounded, true)

  let mutationCount = 0
  const unavailable = Object.assign(new Error("allocator unavailable after restart"), { code: "ECONNREFUSED" })
  await assert.rejects(admission({
    containerName: identity.containerName,
    quotaMarkerPresent: false,
    requestQuota: async () => { throw unavailable },
    run: () => { mutationCount += 1 },
  }), (error) => error === unavailable)
  assert.equal(mutationCount, 0)
})
