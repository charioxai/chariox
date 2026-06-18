import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { runtimeAuthorityMatrixReportFixtures } from "./lib/focused-runtime-fixtures.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-focused-runtime-gate.mjs", import.meta.url))

test("focused runtime gate help lists focused presets", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--help"])

  assert.match(stdout, /runtime-authority/)
  assert.match(stdout, /distributed-state-health/)
  assert.match(stdout, /--matrix-root ROOT/)
  assert.match(stdout, /--require-complete/)
})

test("focused runtime gate accepts option separator forwarding", async () => {
  const { stdout } = await execFile(process.execPath, [
    scriptPath,
    "--matrix-root",
    ".artifacts/drill-matrices",
    "--",
    "--help",
  ])

  assert.match(stdout, /Usage: node apps\/cli\/scripts\/drill-focused-runtime-gate\.mjs/)
})

test("focused runtime gate passes runtime authority and distributed state health presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-focused-runtime-gate-"))
  try {
    const matrixRoot = path.join(rootDir, "matrices")
    const outputPath = path.join(rootDir, "focused-runtime-gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeFocusedRuntimeMatrices(matrixRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--matrix-root",
      matrixRoot,
      "--require-complete",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, report)
    assert.equal(report.schema, "arroba.drill.focused_runtime_gate.v1")
    assert.equal(report.status, "passed")
    assert.deepEqual(report.presets, ["runtime-authority", "distributed-state-health"])
    assert.deepEqual(report.reports.map((entry) => [entry.preset, entry.report.status]), [
      ["runtime-authority", "passed"],
      ["distributed-state-health", "passed"],
    ])
    assert.equal(report.nextActions.length, 0)
    assert.deepEqual(report.reports[0].report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.reports[1].report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.equal(artifactIndex.metadata.artifactKinds, "focused-runtime-gate")
    assert.equal(artifactIndex.metadata.drill, "focused-runtime-gate")
    assert.equal(artifactIndex.metadata.status, "passed")
    assert.equal(artifactIndex.metadata.presets, "runtime-authority,distributed-state-health")
    assert.ok(metadataList(artifactIndex.metadata.runtimeSignals).includes("workspace-live-sync-state"))
    assert.ok(metadataList(artifactIndex.metadata.runtimeSignalOwners).includes("runtime-state"))
    assert.ok(metadataList(artifactIndex.metadata.classifications).includes("workspace-live-sync-conflict"))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("focused runtime gate reports owner-routed gaps from incomplete evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-focused-runtime-gate-"))
  try {
    const matrixRoot = path.join(rootDir, "matrices")
    await mkdir(matrixRoot, { recursive: true })
    await writeJson(path.join(matrixRoot, "remote-agent-runtime.json"), runtimeAuthorityMatrixReportFixtures()[1].report)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--matrix-root",
        matrixRoot,
        "--require-complete",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const report = JSON.parse(error.stdout)
        assert.equal(report.status, "failed")
        assert.equal(report.reports[0].report.status, "failed")
        assert.equal(report.reports[1].report.status, "failed")
        assert.ok(report.nextActions.some((action) => action.preset === "runtime-authority"))
        assert.ok(report.nextActions.some((action) => action.preset === "distributed-state-health"))
        assert.ok(report.reports[1].report.checks.matrices.missingMatrices.includes("workspace-live-sync-matrix"))
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeFocusedRuntimeMatrices(matrixRoot) {
  await mkdir(matrixRoot, { recursive: true })
  for (const fixture of runtimeAuthorityMatrixReportFixtures()) {
    await writeJson(path.join(matrixRoot, fixture.fileName), fixture.report)
  }
  await writeJson(path.join(matrixRoot, "cloud-slice-runtime.json"), matrixReport({
    matrix: "cloud-slice-runtime-matrix",
    deploymentPresets: ["hosted-cloud"],
    providers: ["claude", "codex", "opencode"],
    scenarios: [
      scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
      scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
    ],
  }))
  await writeJson(path.join(matrixRoot, "remote-home-extension.json"), matrixReport({
    matrix: "remote-home-extension-matrix",
    deploymentPresets: ["hetzner", "local", "self-hosted-relay"],
    providers: ["codex"],
    scenarios: [
      scenario("local-single", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health"]),
      scenario("local-collab", "kernel-authority", ["home-extension-manifest-sync", "session-authority"]),
      scenario("hetzner-single", "worker-execution", ["home-extension-manifest-sync", "provider-run-lifecycle"]),
      scenario("hetzner-collab", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health"]),
    ],
  }))
  await writeJson(path.join(matrixRoot, "workspace-live-sync.json"), matrixReport({
    matrix: "workspace-live-sync-matrix",
    deploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
    providers: ["codex", "opencode"],
    scenarios: [
      scenario("local-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-tracked-codex", "kernel-authority", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-managed-codex", "relay-target-freshness", ["relay-target-freshness", "workspace-live-sync-state"]),
      scenario("remote-tracked-codex", "workspace-live-sync-conflict", ["relay-target-freshness", "workspace-live-sync-state"]),
      scenario("remote-tracked-restart-codex", "workspace-live-sync-conflict", ["relay-target-freshness", "workspace-live-sync-state"]),
    ],
  }))
}

function matrixReport({ matrix, deploymentPresets, providers, scenarios }) {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {
      deploymentPresets: deploymentPresets.join(","),
      providers: providers.join(","),
    },
    scenarios,
  }
}

function scenario(id, classification, runtimeSignals) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status: "passed",
    expectedFailure: false,
    classification,
    runtimeSignals,
    durationMs: 10,
    reason: null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
  }
}

async function writeJson(file, value) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(value, null, 2)}\n`, "utf8")
}

function metadataList(value) {
  return String(value ?? "").split(",").filter(Boolean).sort()
}
