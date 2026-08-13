import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { validateDrillChaosReplayBundle } from "./lib/drill-chaos-contract.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./live-runtime-resilience-chaos-matrix-drill.mjs", import.meta.url))

test("runtime resilience chaos matrix dry-run covers local, slice, Hetzner, and hosted recovery axes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-resilience-chaos-matrix-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--include-slices",
      "--include-hetzner",
      "--include-hosted-cloud",
      "--provider-model",
      "codex=gpt-test-codex",
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--chaos-seed",
      "matrix-dry-run",
      "--chaos-replay",
      path.join(rootDir, "replay.json"),
      "--report",
      reportPath,
      "--artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(report.schema, "chariox.drill.matrix.v1")
    assert.equal(report.matrix, "runtime-resilience-chaos-matrix")
    assert.equal(report.status, "dry-run")
    assert.deepEqual(report.scenarios.map((scenario) => scenario.id), [
      "deterministic-runtime-convergence",
      "local-kernel-websocket-drop",
      "local-kernel-restart-durable-state",
      "local-relay-restart-reconnect",
      "local-tui-web-terminal-parity",
      "same-host-remote-worker-restart",
      "worker-provider-resume-codex",
      "worker-provider-resume-opencode",
      "slice-restart-codex",
      "slice-restart-opencode",
      "hetzner-collaborator-reconnect-authority",
      "hosted-cloud-relay-second-kernel-reconnect",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "slice-restart-codex").requires, ["slice"])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "slice-restart-codex").args.slice(-5, -3), [
      "--slice-build-image",
      "always",
    ])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hetzner-collaborator-reconnect-authority").requires, ["hetzner"])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "hosted-cloud-relay-second-kernel-reconnect").requires, ["hosted-cloud"])
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "worker-provider-resume-codex").args.slice(-2), [
      "--provider-model",
      "codex=gpt-test-codex",
    ])
    assert(report.scenarios.find((scenario) => scenario.id === "worker-provider-resume-codex").args.includes("--cleanup-on-success"))
    assert.deepEqual(report.scenarios.find((scenario) => scenario.id === "deterministic-runtime-convergence").args.slice(-4), [
      "--seed",
      "matrix-dry-run",
      "--output",
      path.join(rootDir, "replay.json"),
    ])
    assert.equal(report.metadata.deploymentPresets, "hetzner,hosted-cloud,local,same-host-remote,self-hosted-relay")
    assert.equal(report.metadata.providers, "claude,codex,opencode")
    assert.equal(report.metadata.providerAccountAliases, "claude=work_claude,codex=work_codex")
    assert.equal(report.metadata.includeSlices, true)
    assert.equal(report.metadata.includeHetzner, true)
    assert.equal(report.metadata.includeHostedCloud, true)
    assert.equal(report.metadata.generatedMatrixNames, "runtime-resilience-chaos-matrix")
    assert.equal(report.metadata.generatedMatrixRepos, "oss")
    assert.equal(report.metadata.deterministicChaosSeed, "matrix-dry-run")
    assert.equal(report.metadata.deterministicChaosReplaySchema, "chariox.drill.chaos_replay.v1")
    assert.match(report.metadata.deterministicChaosFaultKinds, /process-death/)
    assert.match(report.metadata.deterministicChaosInvariantIds, /eventual-client-convergence/)
    assert.match(stdout, /dry-run deterministic-runtime-convergence classification=ui-client-projection/)
    assert.match(stdout, /dry-run local-kernel-websocket-drop classification=relay-runtime/)
    assert.match(stdout, /dry-run hosted-cloud-relay-second-kernel-reconnect classification=relay-runtime/)
    assert.equal(artifactIndex.metadata.matrix, "runtime-resilience-chaos-matrix")
    assert.equal(artifactIndex.metadata.dryRun, true)
    assert.equal(artifactIndex.metadata.generatedMatrixNames, "runtime-resilience-chaos-matrix")
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "oss")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("runtime resilience matrix retains a validated deterministic replay on success", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-resilience-replay-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  const replayPath = path.join(rootDir, "replay.json")
  try {
    await execFile(process.execPath, [
      scriptPath,
      "--only",
      "deterministic-runtime-convergence",
      "--chaos-seed",
      "matrix-live-replay",
      "--chaos-replay",
      replayPath,
      "--report",
      reportPath,
      "--artifact-index",
      artifactIndexPath,
    ])
    const replay = JSON.parse(await readFile(replayPath, "utf8"))
    const report = JSON.parse(await readFile(reportPath, "utf8"))
    validateDrillChaosReplayBundle(replay)
    await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(replay.seed, "matrix-live-replay")
    assert.equal(replay.invariants.status, "passed")
    assert.equal(report.status, "passed")
    assert.deepEqual(report.scenarios[0].artifactHints, [replayPath])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("runtime resilience chaos matrix rejects gated scenarios without opt-in flags", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--dry-run", "--only", "slice-restart-codex"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /slice-restart-codex requires --include-slices/)
      return true
    },
  )
})

test("runtime resilience chaos matrix uses the supported Codex default model", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-runtime-resilience-codex-model-"))
  const reportPath = path.join(rootDir, "matrix.json")
  const env = { ...process.env }
  delete env.CHARIOX_RUNTIME_RESILIENCE_CODEX_MODEL
  delete env.CHARIOX_CODEX_MODEL
  try {
    await execFile(process.execPath, [
      scriptPath,
      "--dry-run",
      "--only",
      "worker-provider-resume-codex",
      "--report",
      reportPath,
    ], { env })
    const report = JSON.parse(await readFile(reportPath, "utf8"))

    assert.deepEqual(report.scenarios[0].args.slice(-2), [
      "--provider-model",
      "codex=gpt-5.4-mini",
    ])
    assert.equal(Object.hasOwn(report.metadata, "deterministicChaosSeed"), false)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
