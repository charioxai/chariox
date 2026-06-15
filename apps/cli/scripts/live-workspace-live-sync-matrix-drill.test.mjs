import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { workspaceLiveSyncRequiredScenarioDescriptors } from "./lib/workspace-live-sync-fixtures.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-workspace-live-sync-matrix-drill.mjs", import.meta.url))

test("workspace live sync matrix dry-run covers local remote Hetzner and provider metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-workspace-live-sync-matrix-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--include-remote",
      "--include-hetzner",
      "--include-opencode",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--report",
      reportPath,
      "--artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    const scenarioIds = report.scenarios.map((scenario) => scenario.id)

    assert.equal(report.schema, "arroba.drill.matrix.v1")
    assert.equal(report.matrix, "workspace-live-sync-matrix")
    assert.equal(report.status, "dry-run")
    assert.equal(report.scenarios.length, 20)
    assert(scenarioIds.includes("local-managed-codex"))
    assert(scenarioIds.includes("remote-tracked-codex"))
    assert(scenarioIds.includes("hetzner-tracked-codex"))
    assert(scenarioIds.includes("local-managed-opencode"))
    assert(scenarioIds.includes("remote-tracked-opencode"))
    assert(scenarioIds.includes("hetzner-permission-opencode"))
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hetzner-managed-codex").requires, ["remote", "hetzner"])
    const opencodeRemote = report.scenarios.find((scenario) => scenario.id === "remote-tracked-opencode")
    assert.deepEqual({
      deployment: opencodeRemote.deployment,
      mode: opencodeRemote.mode,
      provider: opencodeRemote.provider,
    }, {
      deployment: "same-host-remote",
      mode: "tracked",
      provider: "opencode",
    })
    const descriptorsById = new Map(workspaceLiveSyncRequiredScenarioDescriptors().map((descriptor) => [descriptor.id, descriptor]))
    for (const scenario of report.scenarios.filter((item) => descriptorsById.has(item.id))) {
      const descriptor = descriptorsById.get(scenario.id)
      assert.equal(scenario.deployment, descriptor.deployment)
      assert.equal(scenario.mode, descriptor.mode)
      assert.equal(scenario.provider, descriptor.provider)
      assert.deepEqual(scenario.requires, descriptor.requires)
    }
    assert.equal(report.metadata.deploymentPresets, "hetzner,local,same-host-remote,self-hosted-relay")
    assert.equal(report.metadata.providerCount, 2)
    assert.equal(report.metadata.providers, "codex,opencode")
    assert.equal(report.metadata.defaultModel, "per-provider")
    assert.equal(report.metadata.providerModelOverrides, "codex,opencode")
    assert.equal(report.metadata.providerAccountAliases, "codex=work_codex,opencode=zen")
    assert.equal(report.metadata.includeRemote, true)
    assert.equal(report.metadata.includeHetzner, true)
    assert.equal(report.metadata.includeOpencode, true)
    assert.match(stdout, /dry-run local-managed-codex classification=workspace-live-sync-conflict/)
    assert.match(stdout, /dry-run remote-tracked-restart-codex classification=relay-target-freshness/)
    assert.match(stdout, /dry-run remote-permission-opencode classification=kernel-authority/)
    assert.equal(artifactIndex.metadata.matrix, "workspace-live-sync-matrix")
    assert.equal(artifactIndex.metadata.dryRun, true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("workspace live sync matrix rejects gated scenarios without opt-in", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "remote-managed-codex"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /remote-managed-codex requires --include-remote/)
      return true
    },
  )

  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "local-managed-opencode"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /local-managed-opencode requires --include-opencode/)
      return true
    },
  )
})
