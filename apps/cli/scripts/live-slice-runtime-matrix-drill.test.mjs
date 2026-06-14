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
const scriptPath = fileURLToPath(new URL("./live-slice-runtime-matrix-drill.mjs", import.meta.url))

test("slice runtime matrix dry-run covers lifecycle auth session agent UI and relay metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-slice-runtime-matrix-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--include-browser-state",
      "--include-self-hosted-relay",
      "--report",
      reportPath,
      "--artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(report.schema, "arroba.drill.matrix.v1")
    assert.equal(report.matrix, "slice-runtime-matrix")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.scenarios.map((scenario) => scenario.id), [
      "slice-lifecycle",
      "provider-auth",
      "session-start",
      "agent-reuse",
      "ui-projection",
      "docker-browser-state",
      "self-hosted-relay-claude-headless",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "docker-browser-state").requires, ["browser-state"])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "self-hosted-relay-claude-headless").requires, ["self-hosted-relay"])
    assert.equal(report.metadata.deploymentPresets, "local,self-hosted-relay")
    assert.equal(report.metadata.providerCount, 3)
    assert.equal(report.metadata.providers, "claude,codex,opencode")
    assert.equal(report.metadata.defaultModel, "provider-default")
    assert.equal(report.metadata.providerModelOverrides, "")
    assert.equal(report.metadata.includeBrowserState, true)
    assert.equal(report.metadata.includeSelfHostedRelay, true)
    assert.match(stdout, /dry-run slice-lifecycle classification=slice-runtime/)
    assert.match(stdout, /dry-run provider-auth classification=slice-auth/)
    assert.match(stdout, /dry-run session-start classification=kernel-authority/)
    assert.match(stdout, /dry-run docker-browser-state classification=docker-runtime/)
    assert.match(stdout, /dry-run self-hosted-relay-claude-headless classification=worker-execution/)
    assert.equal(artifactIndex.metadata.matrix, "slice-runtime-matrix")
    assert.equal(artifactIndex.metadata.dryRun, true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("slice runtime matrix rejects opt-in scenarios without their flags", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "docker-browser-state"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /docker-browser-state requires --include-browser-state/)
      return true
    },
  )

  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "self-hosted-relay-claude-headless"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /self-hosted-relay-claude-headless requires --include-self-hosted-relay/)
      return true
    },
  )
})
