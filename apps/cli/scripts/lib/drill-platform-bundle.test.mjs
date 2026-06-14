import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DRILL_PLATFORM_BUNDLE_ARTIFACTS,
  DRILL_PLATFORM_BUNDLE_SCHEMA,
  verifyDrillPlatformBundle,
  writeDrillPlatformBundle,
} from "./drill-platform-bundle.mjs"

test("defines stable drill platform bundle artifacts", () => {
  assert.deepEqual(DRILL_PLATFORM_BUNDLE_ARTIFACTS, [
    {
      path: "failure-taxonomy-drill.json",
      schema: "arroba.drill.failure_taxonomy.v1",
    },
    {
      path: "failure-taxonomy-scenario.json",
      schema: "arroba.drill.failure_taxonomy.v1",
    },
    {
      path: "validation-suite.json",
      schema: "arroba.drill.validation_suite.v1",
    },
  ])
})

test("writes and verifies drill platform bundle artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    const bundle = await writeDrillPlatformBundle(rootDir)
    const verified = await verifyDrillPlatformBundle(rootDir)
    const validationSuite = JSON.parse(await readFile(path.join(rootDir, "validation-suite.json"), "utf8"))

    assert.equal(bundle.schema, DRILL_PLATFORM_BUNDLE_SCHEMA)
    assert.deepEqual(verified, bundle)
    assert.equal(validationSuite.schema, "arroba.drill.validation_suite.v1")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unsafe drill platform bundle artifact paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeFile(path.join(rootDir, "index.json"), `${JSON.stringify({
      schema: DRILL_PLATFORM_BUNDLE_SCHEMA,
      outputDir: rootDir,
      artifacts: [{ path: "../outside.json", schema: "arroba.drill.validation_suite.v1" }],
    })}\n`, "utf8")

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /unsafe artifact path/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects incomplete drill platform bundles", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeFile(path.join(rootDir, "index.json"), `${JSON.stringify({
      schema: DRILL_PLATFORM_BUNDLE_SCHEMA,
      outputDir: rootDir,
      artifacts: [{
        path: "validation-suite.json",
        schema: "arroba.drill.validation_suite.v1",
        sha256: "0".repeat(64),
        sizeBytes: 0,
      }],
    })}\n`, "utf8")

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /artifacts do not match required platform contracts/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite artifact count drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", { ...suite, testCount: suite.testCount + 1 })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /testCount does not match testPaths/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects failure taxonomy target drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const taxonomyPath = path.join(rootDir, "failure-taxonomy-drill.json")
    const taxonomy = JSON.parse(await readFile(taxonomyPath, "utf8"))
    await replaceBundleArtifact(rootDir, "failure-taxonomy-drill.json", { ...taxonomy, target: "scenario" })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /target mismatch/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects failure taxonomy classification drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const taxonomyPath = path.join(rootDir, "failure-taxonomy-scenario.json")
    const taxonomy = JSON.parse(await readFile(taxonomyPath, "utf8"))
    await replaceBundleArtifact(rootDir, "failure-taxonomy-scenario.json", {
      ...taxonomy,
      classifications: taxonomy.classifications.slice(1),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /classifications do not match taxonomy/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function replaceBundleArtifact(rootDir, artifactPath, contents) {
  const serialized = `${JSON.stringify(contents, null, 2)}\n`
  await writeFile(path.join(rootDir, artifactPath), serialized, "utf8")
  const indexPath = path.join(rootDir, "index.json")
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  index.artifacts = index.artifacts.map((artifact) => artifact.path === artifactPath
    ? {
        ...artifact,
        sha256: createHash("sha256").update(serialized).digest("hex"),
        sizeBytes: Buffer.byteLength(serialized),
      }
    : artifact)
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
}
