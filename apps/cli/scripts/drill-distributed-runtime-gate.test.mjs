import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-distributed-runtime-gate.mjs", import.meta.url))

test("distributed runtime gate passes with complete OSS and Cloud matrix evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const outputPath = path.join(rootDir, "gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
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
    assert.equal(report.status, "passed")
    assert.deepEqual(report.presets, ["distributed-runtime"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingProviders, [])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 1)
    assert.equal(artifactIndex.metadata.drill, "distributed-runtime-gate")
    assert.equal(artifactIndex.metadata.preset, "distributed-runtime")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate reports missing hosted Cloud evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: false })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        assert.deepEqual(report.checks.matrices.missingScenarios, ["ui-projection"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

async function writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud }) {
  const ossMatrixRoot = path.join(ossRoot, ".artifacts", "drill-matrices")
  await writeMatrixReport(path.join(ossMatrixRoot, "native-provider-tui.json"), {
    matrix: "native-provider-tui-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("local-native-tui", "kernel-authority"),
      scenario("permission-visibility", "ui-client-projection"),
      scenario("remote-native-tui", "relay-runtime"),
      scenario("slice-native-tui", "worker-execution"),
      scenario("transcript-parity", "provider-error"),
      scenario("provider-auth-health", "provider-auth"),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-agent-runtime.json"), {
    matrix: "remote-agent-runtime-matrix",
    metadata: {
      deploymentPresets: "hetzner,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("collab-remote-agent", "kernel-authority"),
      scenario("lease-reconnect", "relay-target-freshness"),
      scenario("provider-run-binding", "worker-execution"),
      scenario("remote-prompt-dispatch", "relay-runtime"),
      scenario("single-user-remote-agent", "ui-client-projection"),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-home-extension.json"), {
    matrix: "remote-home-extension-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,self-hosted-relay",
    },
    scenarios: [
      scenario("local-single", "remote-extension-sync"),
      scenario("local-collab", "kernel-authority"),
      scenario("hetzner-single", "worker-execution"),
      scenario("hetzner-collab", "kernel-authority"),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "slice-runtime.json"), {
    matrix: "slice-runtime-matrix",
    metadata: {
      deploymentPresets: "local,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("agent-reuse", "worker-execution"),
      scenario("docker-browser-state", "docker-runtime"),
      scenario("provider-auth", "slice-auth"),
      scenario("session-start", "kernel-authority"),
      scenario("slice-lifecycle", "slice-runtime"),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "workspace-live-sync.json"), {
    matrix: "workspace-live-sync-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "codex,opencode",
    },
    scenarios: [
      scenario("local-managed-codex", "workspace-live-sync-conflict"),
      scenario("local-tracked-codex", "workspace-live-sync-conflict"),
      scenario("local-permission-codex", "kernel-authority"),
      scenario("remote-managed-codex", "workspace-live-sync-conflict"),
      scenario("remote-tracked-codex", "workspace-live-sync-conflict"),
      scenario("remote-tracked-restart-codex", "relay-target-freshness"),
    ],
  })

  if (includeCloud) {
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection"),
      ],
    })
  }
}

async function writeMatrixReport(file, { matrix, metadata, scenarios }) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios,
  }, null, 2)}\n`, "utf8")
}

function scenario(id, classification) {
  return {
    id,
    description: `${id} scenario`,
    status: "passed",
    ok: true,
    expectedFailure: false,
    classification,
    durationMs: 10,
    reason: null,
    requires: [],
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    exitCriteria: [`${id} exit criteria`],
  }
}
