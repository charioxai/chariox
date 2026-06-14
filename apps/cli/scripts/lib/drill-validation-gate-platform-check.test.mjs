import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { writeDrillPlatformBundle } from "./drill-platform-bundle.mjs"
import { platformValidationGateCheck } from "./drill-validation-gate-platform-check.mjs"

test("skips platform bundle validation when no bundle or requirements are configured", async () => {
  const check = await platformValidationGateCheck(null)

  assert.deepEqual(check, {
    status: "skipped",
    dir: null,
    requiredCoverageAreas: [],
    missingCoverageAreas: [],
    requiredFailureClassifications: [],
    missingFailureClassifications: [],
  })
})

test("fails when platform evidence is required but no bundle is provided", async () => {
  const check = await platformValidationGateCheck(null, {
    requiredCoverageAreas: ["matrix-validation"],
    requiredFailureClassifications: ["kernel-authority"],
  })

  assert.deepEqual(check, {
    status: "failed",
    dir: null,
    requiredCoverageAreas: ["matrix-validation"],
    missingCoverageAreas: ["matrix-validation"],
    requiredFailureClassifications: ["kernel-authority"],
    missingFailureClassifications: ["kernel-authority"],
    error: "no platform bundle provided",
  })
})

test("passes with platform bundle coverage and failure taxonomy evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-platform-"))
  try {
    await writeDrillPlatformBundle(rootDir)

    const check = await platformValidationGateCheck(rootDir, {
      requiredCoverageAreas: ["matrix-validation"],
      requiredFailureClassifications: ["kernel-authority", "workspace-live-sync-conflict"],
    })

    assert.equal(check.status, "passed")
    assert.equal(check.dir, rootDir)
    assert.deepEqual(check.missingCoverageAreas, [])
    assert.deepEqual(check.missingFailureClassifications, [])
    assert.equal(check.validationSuite.coverageAreas.some((area) => area.id === "matrix-validation"), true)
    assert.deepEqual(check.validationSuite.validationPresets.map((preset) => preset.name), [
      "distributed-runtime",
      "native-provider-tui",
      "remote-agent-runtime",
      "remote-home-extension",
      "slice-runtime",
      "workspace-live-sync",
    ])
    assert.deepEqual(
      check.validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredMatrices,
      ["native-provider-tui-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
    )
    assert.deepEqual(
      check.validationSuite.validationPresets.find((preset) => preset.name === "workspace-live-sync").requiredMatrices,
      ["workspace-live-sync-matrix"],
    )
    assert.equal(check.failureTaxonomy.drill.includes("kernel-authority"), true)
    assert.equal(check.failureTaxonomy.scenario.includes("workspace-live-sync-conflict"), true)
    assert.equal(check.artifacts.length > 0, true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when platform bundle misses required coverage dimensions", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-platform-"))
  try {
    await writeDrillPlatformBundle(rootDir)

    const check = await platformValidationGateCheck(rootDir, {
      requiredCoverageAreas: ["hosted-cloud-drills"],
      requiredFailureClassifications: ["unknown-diagnostic-class"],
    })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.missingCoverageAreas, ["hosted-cloud-drills"])
    assert.deepEqual(check.missingFailureClassifications, ["unknown-diagnostic-class"])
    assert.match(check.error, /missing platform coverage areas: hosted-cloud-drills/)
    assert.match(check.error, /missing failure classifications: unknown-diagnostic-class/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when platform bundle taxonomy evidence is tampered", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-platform-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    await writeFile(path.join(rootDir, "failure-taxonomy-drill.json"), JSON.stringify({
      schema: "arroba.drill.failure_taxonomy.v1",
      target: "scenario",
      classifications: [],
    }), "utf8")

    const check = await platformValidationGateCheck(rootDir, {
      requiredFailureClassifications: ["kernel-authority"],
    })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.missingFailureClassifications, ["kernel-authority"])
    assert.match(check.error, /platform bundle artifact failure-taxonomy-drill\.json sha256 mismatch/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
