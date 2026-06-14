import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
import { artifactValidationGateCheck } from "./drill-validation-gate-artifact-check.mjs"

test("skips artifact validation when no roots or indexes are configured", async () => {
  const check = await artifactValidationGateCheck({
    artifactIndexes: [],
    artifactRoots: [],
  }, { maxDepth: 8 })

  assert.deepEqual(check, {
    status: "skipped",
    roots: [],
    inputs: [],
    indexPaths: [],
    requiredArtifactSchemas: [],
    missingArtifactSchemas: [],
  })
})

test("fails when required artifact schemas have no evidence", async () => {
  const check = await artifactValidationGateCheck({
    artifactIndexes: [],
    artifactRoots: [],
  }, {
    maxDepth: 8,
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
  })

  assert.equal(check.status, "failed")
  assert.deepEqual(check.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.deepEqual(check.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.match(check.error, /missing required artifact schemas/)
})

test("fails when artifact roots contain no indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    const check = await artifactValidationGateCheck({
      artifactIndexes: [],
      artifactRoots: [rootDir],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.roots, [rootDir])
    assert.deepEqual(check.indexPaths, [])
    assert.equal(check.error, "no artifact indexes found")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("passes with explicit and discovered artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    const explicitRoot = path.join(rootDir, "explicit")
    const discoveredRoot = path.join(rootDir, "discovered")
    const explicitIndex = path.join(explicitRoot, "custom-index.json")
    const discoveredIndex = path.join(discoveredRoot, "arroba-drill-artifacts.json")
    await writeReportArtifact(explicitRoot, "reports/gate.json")
    await writeReportArtifact(discoveredRoot, "reports/matrix.json")
    await writeDrillArtifactIndex({
      rootDir: explicitRoot,
      artifacts: ["reports/gate.json"],
      indexPath: explicitIndex,
      metadata: { drill: "explicit" },
    })
    await writeDrillArtifactIndex({
      rootDir: discoveredRoot,
      artifacts: ["reports/matrix.json"],
      metadata: { drill: "discovered" },
    })

    const check = await artifactValidationGateCheck({
      artifactIndexes: [explicitIndex],
      artifactRoots: [rootDir],
    }, { maxDepth: 8 })

    assert.equal(check.status, "passed")
    assert.deepEqual(check.inputs, [explicitIndex])
    assert.deepEqual(check.indexPaths, [explicitIndex, discoveredIndex].sort())
    assert.equal(check.aggregate.totals.indexes, 2)
    assert.equal(check.aggregate.totals.artifacts, 2)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates required artifact schemas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: { drill: "schema-gate" },
    })

    const pass = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactSchemas: ["arroba.drill.validation_gate.v1"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.requiredArtifactSchemas, ["arroba.drill.validation_gate.v1"])
    assert.deepEqual(pass.missingArtifactSchemas, [])

    const fail = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.match(fail.error, /missing required artifact schemas/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when an indexed artifact is tampered", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    const artifactPath = path.join(rootDir, "reports", "gate.json")
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    await writeFile(artifactPath, "{\"schema\":\"tampered\"}\n", "utf8")

    const check = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.match(check.error, /drill artifact reports\/gate\.json sha256 mismatch/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeReportArtifact(rootDir, relativePath) {
  const file = path.join(rootDir, relativePath)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.validation_gate.v1",
    status: "passed",
  })}\n`, "utf8")
}
