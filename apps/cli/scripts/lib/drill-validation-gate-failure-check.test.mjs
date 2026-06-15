import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { failureValidationGateCheck } from "./drill-validation-gate-failure-check.mjs"

test("skips failure validation when no roots or inputs are configured", async () => {
  const check = await failureValidationGateCheck({
    failureInputs: [],
    failureRoots: [],
  }, { maxDepth: 8 })

  assert.deepEqual(check, {
    status: "skipped",
    roots: [],
    inputs: [],
    manifestPaths: [],
    requiredFailureMaxAgeMs: null,
    staleFailureManifests: [],
  })
})

test("fails and aggregates explicit failure manifest directories", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-failures-"))
  try {
    const runDir = path.join(rootDir, "failed-run")
    const manifestPath = path.join(runDir, "arroba-drill-failure.json")
    await writeFailureManifest(manifestPath, { drill: "provider-drill" })

    const check = await failureValidationGateCheck({
      failureInputs: [runDir],
      failureRoots: [],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.inputs, [runDir])
    assert.deepEqual(check.manifestPaths, [manifestPath])
    assert.equal(check.aggregate.total, 1)
    assert.deepEqual(check.aggregate.classifications, { "provider-auth": 1 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("discovers failure manifests below configured roots", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-failures-"))
  try {
    const first = path.join(rootDir, "target", "run-one", "arroba-drill-failure.json")
    const second = path.join(rootDir, ".artifacts", "run-two", "arroba-drill-failure.json")
    await writeFailureManifest(first, { drill: "first", message: "relay target stale" })
    await writeFailureManifest(second, { drill: "second", message: "workspace live sync result skipped_conflict" })

    const check = await failureValidationGateCheck({
      failureInputs: [],
      failureRoots: [rootDir],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.manifestPaths, [second, first].sort())
    assert.equal(check.aggregate.total, 2)
    assert.deepEqual(check.aggregate.owners, {
      "runtime-network": 1,
      "runtime-state": 1,
    })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("deduplicates manifests supplied by both inputs and roots", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-failures-"))
  try {
    const runDir = path.join(rootDir, "failed-run")
    const manifestPath = path.join(runDir, "arroba-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const check = await failureValidationGateCheck({
      failureInputs: [runDir],
      failureRoots: [rootDir],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.manifestPaths, [manifestPath])
    assert.equal(check.aggregate.total, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails with diagnostic error when a failure manifest is malformed", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-failures-"))
  try {
    const manifestPath = path.join(rootDir, "arroba-drill-failure.json")
    await mkdir(path.dirname(manifestPath), { recursive: true })
    await writeFile(manifestPath, "{\"schema\":\"wrong\"}\n", "utf8")

    const check = await failureValidationGateCheck({
      failureInputs: [manifestPath],
      failureRoots: [],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.manifestPaths, [])
    assert.match(check.error, /unsupported schema/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates preserved failure manifests by required freshness", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-failures-"))
  try {
    const manifestPath = path.join(rootDir, "arroba-drill-failure.json")
    const failedAt = new Date(Date.now() - 500).toISOString()
    await writeFailureManifest(manifestPath, {
      drill: "stale-failure",
      failedAt,
    })

    const fresh = await failureValidationGateCheck({
      failureInputs: [manifestPath],
      failureRoots: [],
    }, {
      maxDepth: 8,
      requiredFailureMaxAgeMs: 3_600_000,
    })
    assert.equal(fresh.requiredFailureMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.staleFailureManifests, [])

    const stale = await failureValidationGateCheck({
      failureInputs: [manifestPath],
      failureRoots: [],
    }, {
      maxDepth: 8,
      requiredFailureMaxAgeMs: 100,
    })
    assert.equal(stale.status, "failed")
    assert.equal(stale.staleFailureManifests.length, 1)
    assert.equal(stale.staleFailureManifests[0].source, manifestPath)
    assert.equal(stale.staleFailureManifests[0].drill, "stale-failure")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeFailureManifest(file, {
  drill = "failed-drill",
  failedAt = "2026-06-13T00:00:00.000Z",
  message = "Token refresh failed: 401",
} = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt,
    metadata: { drill },
    error: { name: "Error", message, stack: null },
  }, null, 2)}\n`, "utf8")
}
