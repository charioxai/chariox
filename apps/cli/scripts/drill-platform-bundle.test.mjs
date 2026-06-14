import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-platform-bundle.mjs", import.meta.url))

test("drill platform bundle writes shared contract artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-"))
  const outputDir = path.join(rootDir, "bundle")
  try {
    const { stdout } = await execFile(process.execPath, [scriptPath, "--output-dir", outputDir])
    const stdoutBundle = JSON.parse(stdout)
    const indexBundle = JSON.parse(await readFile(path.join(outputDir, "index.json"), "utf8"))
    const validationSuite = JSON.parse(await readFile(path.join(outputDir, "validation-suite.json"), "utf8"))
    const scenarioTaxonomy = JSON.parse(await readFile(path.join(outputDir, "failure-taxonomy-scenario.json"), "utf8"))
    const drillTaxonomy = JSON.parse(await readFile(path.join(outputDir, "failure-taxonomy-drill.json"), "utf8"))

    assert.deepEqual(indexBundle, stdoutBundle)
    assert.equal(indexBundle.schema, "arroba.drill.platform_bundle.v1")
    assert.deepEqual(indexBundle.artifacts.map(({ path, schema }) => ({ path, schema })), [
      {
        path: "validation-suite.json",
        schema: "arroba.drill.validation_suite.v1",
      },
      {
        path: "failure-taxonomy-scenario.json",
        schema: "arroba.drill.failure_taxonomy.v1",
      },
      {
        path: "failure-taxonomy-drill.json",
        schema: "arroba.drill.failure_taxonomy.v1",
      },
    ])
    for (const artifact of indexBundle.artifacts) {
      assert.match(artifact.sha256, /^[a-f0-9]{64}$/)
      assert(Number.isSafeInteger(artifact.sizeBytes))
      const serialized = await readFile(path.join(outputDir, artifact.path), "utf8")
      assert.equal(artifact.sha256, createHash("sha256").update(serialized).digest("hex"))
      assert.equal(artifact.sizeBytes, Buffer.byteLength(serialized))
    }
    assert.equal(validationSuite.schema, "arroba.drill.validation_suite.v1")
    assert.equal(scenarioTaxonomy.target, "scenario")
    assert.equal(drillTaxonomy.target, "drill")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill platform bundle verifies written artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-"))
  const outputDir = path.join(rootDir, "bundle")
  try {
    await execFile(process.execPath, [scriptPath, "--output-dir", outputDir])
    const { stdout } = await execFile(process.execPath, [scriptPath, "--verify-dir", outputDir])
    const verified = JSON.parse(stdout)

    assert.equal(verified.schema, "arroba.drill.platform_bundle.v1")
    assert.equal(verified.artifacts.length, 3)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill platform bundle rejects artifact integrity drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-"))
  const outputDir = path.join(rootDir, "bundle")
  try {
    await execFile(process.execPath, [scriptPath, "--output-dir", outputDir])
    await writeFile(
      path.join(outputDir, "validation-suite.json"),
      `${JSON.stringify({ schema: "wrong.schema" })}\n`,
      "utf8",
    )

    await assert.rejects(
      execFile(process.execPath, [scriptPath, "--verify-dir", outputDir]),
      /sha256 mismatch/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
