import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DRILL_PLATFORM_BUNDLE_SCHEMA,
  verifyDrillPlatformBundle,
  writeDrillPlatformBundle,
} from "./drill-platform-bundle.mjs"

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
