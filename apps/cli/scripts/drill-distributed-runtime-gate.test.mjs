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
      "--require-runtime-signal",
      "slice-auth-state",
      "--require-matrix-runtime-signal",
      "slice-auth-state",
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
    assert.deepEqual(report.checks.platformBundle.missingRuntimeSignals, [])
    assert.ok(report.checks.platformBundle.requiredRuntimeSignals.includes("slice-auth-state"))
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.ok(report.checks.matrices.requiredMatrixRuntimeSignals.includes("slice-auth-state"))
    assert.deepEqual(report.checks.matrices.aggregate.runtimeSignalScenarios["slice-auth-state"].map((entry) => entry.id), ["provider-auth"])
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.deepEqual(
      report.checks.matrices.aggregate.reports.find((entry) => entry.matrix === "cloud-slice-runtime-matrix").providers,
      ["claude", "codex", "opencode"],
    )
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 1)
    assert.equal(artifactIndex.metadata.drill, "distributed-runtime-gate")
    assert.equal(artifactIndex.metadata.preset, "distributed-runtime")
    const indexedRuntimeSignals = artifactIndex.metadata.runtimeSignals.split(",")
    assert.equal(indexedRuntimeSignals.includes("home-extension-manifest-sync"), true)
    assert.equal(indexedRuntimeSignals.includes("provider-run-lifecycle"), true)
    assert.equal(indexedRuntimeSignals.includes("slice-auth-state"), true)
    assert.equal(indexedRuntimeSignals.includes("workspace-live-sync-state"), true)
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

test("distributed runtime gate rejects requirement flags without values", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-runtime-signal", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-runtime-signal requires a value/)
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
      scenario("local-native-tui", "kernel-authority", ["provider-run-lifecycle", "session-authority"]),
      scenario("permission-visibility", "ui-client-projection", ["permission-interaction"]),
      scenario("remote-native-tui", "relay-runtime", ["provider-run-lifecycle", "session-authority"]),
      scenario("slice-native-tui", "worker-execution", ["provider-run-lifecycle", "session-authority"]),
      scenario("transcript-parity", "provider-error", ["client-projection-health"]),
      scenario("provider-auth-health", "provider-auth", ["provider-run-lifecycle"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-agent-runtime.json"), {
    matrix: "remote-agent-runtime-matrix",
    metadata: {
      deploymentPresets: "hetzner,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
      scenario("lease-reconnect", "relay-target-freshness", ["lease-health", "relay-target-freshness"]),
      scenario("provider-run-binding", "worker-execution", ["lease-health", "provider-run-lifecycle"]),
      scenario("remote-prompt-dispatch", "relay-runtime", ["agent-lifecycle", "provider-run-lifecycle"]),
      scenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "session-authority"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-home-extension.json"), {
    matrix: "remote-home-extension-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,self-hosted-relay",
    },
    scenarios: [
      scenario("local-single", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("local-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
      scenario("hetzner-single", "worker-execution", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("hetzner-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "slice-runtime.json"), {
    matrix: "slice-runtime-matrix",
    metadata: {
      deploymentPresets: "local,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("agent-reuse", "worker-execution", ["agent-lifecycle", "slice-runtime-state"]),
      scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
      scenario("session-start", "kernel-authority", ["session-authority", "slice-runtime-state"]),
      scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "workspace-live-sync.json"), {
    matrix: "workspace-live-sync-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "codex,opencode",
    },
    scenarios: [
      scenario("local-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-permission-codex", "kernel-authority", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-tracked-restart-codex", "relay-target-freshness", ["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
    ],
  })

  if (includeCloud) {
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection", ["client-projection-health"], { providers: ["claude", "codex", "opencode"] }),
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

function scenario(id, classification, runtimeSignals = [], overrides = {}) {
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
    runtimeSignals,
    ...overrides,
  }
}
