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
const scriptPath = fileURLToPath(new URL("./live-remote-agent-runtime-matrix-drill.mjs", import.meta.url))

test("remote agent runtime matrix dry-run covers required scenarios and deployment metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-remote-agent-runtime-matrix-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--include-hetzner",
      "--include-hosted-cloud",
      "--provider-model",
      "claude=sonnet-test",
      "--report",
      reportPath,
      "--artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    const scenarioIds = report.scenarios.map((scenario) => scenario.id)

    assert.equal(report.schema, "arroba.drill.matrix.v1")
    assert.equal(report.matrix, "remote-agent-runtime-matrix")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(scenarioIds, [
      "single-user-remote-agent",
      "remote-prompt-dispatch",
      "provider-run-binding",
      "provider-auth-health",
      "ui-client-projection",
      "lease-reconnect",
      "collab-remote-agent",
      "hetzner-single-user-remote-agent",
      "hetzner-collab-remote-agent",
      "hosted-single-user-remote-agent",
      "hosted-collab-remote-agent",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "provider-run-binding").args.slice(1, 5), [
      "--provider",
      "claude-headless",
      "--provider-model",
      "claude-headless=sonnet-test",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hosted-single-user-remote-agent").requires, ["hosted-cloud"])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hetzner-single-user-remote-agent").requires, ["hetzner"])
    assert.equal(report.metadata.deploymentPresets, "hetzner,hosted-cloud,same-host-remote,self-hosted-relay")
    assert.equal(report.metadata.providers, "claude,codex,opencode")
    assert.equal(report.metadata.includesHetzner, true)
    assert.equal(report.metadata.includesHostedCloud, true)
    assert.equal(report.metadata.includesSelfHostedRelay, true)
    assert.match(stdout, /dry-run provider-run-binding classification=provider-error/)
    assert.match(stdout, /dry-run hosted-collab-remote-agent classification=kernel-authority/)
    assert.equal(artifactIndex.metadata.matrix, "remote-agent-runtime-matrix")
    assert.equal(artifactIndex.metadata.dryRun, true)
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "matrix.json",
      schema: "arroba.drill.matrix.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("remote agent runtime matrix rejects gated scenarios without opt-in flags", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "hosted-single-user-remote-agent"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /hosted-single-user-remote-agent requires --include-hosted-cloud/)
      return true
    },
  )
})
