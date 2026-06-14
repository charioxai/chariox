import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
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
    assert.deepEqual(indexBundle.artifacts, [
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
    assert.equal(validationSuite.schema, "arroba.drill.validation_suite.v1")
    assert.equal(scenarioTaxonomy.target, "scenario")
    assert.equal(drillTaxonomy.target, "drill")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
