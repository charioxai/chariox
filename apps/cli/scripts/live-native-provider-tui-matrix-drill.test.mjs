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
const scriptPath = fileURLToPath(new URL("./live-native-provider-tui-matrix-drill.mjs", import.meta.url))

test("native provider TUI matrix dry-run covers authority placement and provider metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-native-provider-tui-matrix-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
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

    assert.equal(report.schema, "arroba.drill.matrix.v1")
    assert.equal(report.matrix, "native-provider-tui-matrix")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.scenarios.map((scenario) => scenario.id), [
      "local-native-tui",
      "permission-visibility",
      "provider-auth-health",
      "remote-native-tui",
      "slice-native-tui",
      "transcript-parity",
      "hetzner-native-tui",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hetzner-native-tui").requires, ["hetzner"])
    assert.equal(report.metadata.deploymentPresets, "hetzner,local,same-host-remote,self-hosted-relay")
    assert.equal(report.metadata.providerCount, 3)
    assert.equal(report.metadata.providers, "claude,codex,opencode")
    assert.equal(report.metadata.defaultModel, "provider-default")
    assert.equal(report.metadata.providerModelOverrides, "")
    assert.equal(report.metadata.includeHetzner, true)
    assert.match(stdout, /dry-run local-native-tui classification=provider-error/)
    assert.match(stdout, /dry-run permission-visibility classification=kernel-authority/)
    assert.match(stdout, /dry-run provider-auth-health classification=provider-auth/)
    assert.match(stdout, /dry-run transcript-parity classification=ui-client-projection/)
    assert.equal(artifactIndex.metadata.matrix, "native-provider-tui-matrix")
    assert.equal(artifactIndex.metadata.dryRun, true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("native provider TUI matrix rejects Hetzner scenario without opt-in", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "hetzner-native-tui"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /hetzner-native-tui requires --include-hetzner/)
      return true
    },
  )
})
