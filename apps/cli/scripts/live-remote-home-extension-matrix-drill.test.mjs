import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-remote-home-extension-matrix-drill.mjs", import.meta.url))

test("remote home extension matrix dry-run covers local and Hetzner authority scenarios", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-remote-home-extension-matrix-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--include-hetzner",
      "--report",
      reportPath,
      "--artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(report.schema, "chariox.drill.matrix.v1")
    assert.equal(report.matrix, "remote-home-extension-matrix")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.scenarios.map((scenario) => scenario.id), [
      "local-single",
      "local-collab",
      "hetzner-single",
      "hetzner-collab",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hetzner-single").requires, ["hetzner"])
    assert.equal(report.metadata.deploymentPresets, "hetzner,local,self-hosted-relay")
    assert.equal(report.metadata.includesHetzner, true)
    assert.equal(report.metadata.includesSelfHostedRelay, true)
    assert.equal(report.metadata.generatedMatrixNames, "remote-home-extension-matrix")
    assert.equal(report.metadata.generatedMatrixRepos, "oss")
    assert.match(stdout, /dry-run local-single classification=remote-extension-sync/)
    assert.match(stdout, /dry-run hetzner-collab classification=kernel-authority/)
    assert.equal(artifactIndex.metadata.matrix, "remote-home-extension-matrix")
    assert.equal(artifactIndex.metadata.dryRun, true)
    assert.equal(artifactIndex.metadata.generatedMatrixNames, "remote-home-extension-matrix")
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "oss")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("remote home extension matrix rejects Hetzner scenarios without opt-in", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "hetzner-single"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /hetzner-single requires --include-hetzner/)
      return true
    },
  )
})
