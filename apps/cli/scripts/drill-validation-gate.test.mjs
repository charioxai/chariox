import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-validation-gate.mjs", import.meta.url))

test("drill validation gate writes passing JSON report", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  const outputPath = path.join(rootDir, "gate.json")
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--platform-bundle",
      bundleDir,
      "--json",
      "--output",
      outputPath,
    ])
    const stdoutReport = JSON.parse(stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))

    assert.deepEqual(fileReport, stdoutReport)
    assert.equal(fileReport.status, "passed")
    assert.equal(fileReport.checks.platformBundle.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate exits non-zero for preserved failures", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    await writeFailureManifest(path.join(rootDir, "failed", "arroba-drill-failure.json"))

    await assert.rejects(
      execFile(process.execPath, [scriptPath, "--failure-root", rootDir, "--json"]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "provider-account", classification: "provider-auth" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeFailureManifest(file) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill: "failed-drill" },
    error: { name: "Error", message: "Token refresh failed: 401", stack: null },
  }, null, 2)}\n`, "utf8")
}
