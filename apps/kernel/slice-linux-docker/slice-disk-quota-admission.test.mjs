import assert from "node:assert/strict"
import { test } from "node:test"
import { runWithSliceDiskQuotaAdmission } from "./slice-disk-quota-admission.mjs"

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
