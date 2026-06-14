import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import {
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
} from "./lib/drill-artifacts.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-validation-gate.mjs", import.meta.url))

test("drill validation gate writes passing JSON report", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  const outputPath = path.join(rootDir, "gate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--platform-bundle",
      bundleDir,
      "--require-failure-classification",
      "kernel-authority,remote-extension-sync",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutReport = JSON.parse(stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, stdoutReport)
    assert.equal(fileReport.status, "passed")
    assert.equal(fileReport.checks.platformBundle.status, "passed")
    assert.deepEqual(fileReport.checks.platformBundle.requiredFailureClassifications, ["kernel-authority", "remote-extension-sync"])
    assert.deepEqual(fileReport.checks.platformBundle.missingFailureClassifications, [])
    assert.equal(artifactIndex.metadata.drill, "validation-gate")
    assert.equal(artifactIndex.metadata.status, "passed")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "gate.json",
      schema: "arroba.drill.validation_gate.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate rejects unknown required failure classifications", async () => {
  await assert.rejects(
    execFile(process.execPath, [
      scriptPath,
      "--require-failure-classification",
      "kernel-authority,remote-extension-synch",
      "--json",
    ]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /unknown required failure classification: remote-extension-synch/)
      return true
    },
  )
})

test("drill validation gate rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("drill validation gate lists presets", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--list-presets"])

  assert.match(stdout, /validation gate presets:/)
  assert.match(stdout, /distributed-runtime/)
  assert.match(stdout, /native-provider-tui/)
  assert.match(stdout, /workspace-live-sync/)
  assert.match(stdout, /remote-home-extension/)
  assert.match(stdout, /matrix_classifications=kernel-authority,relay-target-freshness,workspace-live-sync-conflict/)
})

test("drill validation gate help lists presets from the registry", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--help"])

  assert.match(stdout, /Known: distributed-runtime, native-provider-tui, remote-agent-runtime, remote-home-extension, slice-runtime, workspace-live-sync/)
})

test("drill validation gate lists selected presets as JSON", async () => {
  const { stdout } = await execFile(process.execPath, [
    scriptPath,
    "--list-presets",
    "--preset",
    "remote-home-extension",
    "--json",
  ])
  const parsed = JSON.parse(stdout)

  assert.deepEqual(parsed.presets.map((preset) => preset.name), ["remote-home-extension"])
  assert.deepEqual(parsed.presets[0].requiredMatrices, ["remote-home-extension-matrix"])
  assert.deepEqual(parsed.presets[0].requiredFailureClassifications, ["kernel-authority", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "worker-execution"])
})

test("drill validation gate accepts explicit matrix report paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--matrix-report",
      reportPath,
      "--require-complete",
      "--json",
    ])
    const report = JSON.parse(stdout)

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.inputs, [reportPath])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate requires platform coverage areas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--platform-bundle",
        bundleDir,
        "--require-platform-coverage-area",
        "runtime-fixtures,hosted-cloud-drills",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.platformBundle.requiredCoverageAreas, ["hosted-cloud-drills", "runtime-fixtures"])
        assert.deepEqual(report.checks.platformBundle.missingCoverageAreas, ["hosted-cloud-drills"])
        assert.match(report.checks.platformBundle.error, /missing platform coverage areas: hosted-cloud-drills/)
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "platform-bundle" },
          { owner: "validation-harness", classification: "platform-bundle" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate applies requirement presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "workspace-live-sync.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeWorkspaceLiveSyncPresetReport(reportPath)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--platform-bundle",
      bundleDir,
      "--matrix-report",
      reportPath,
      "--preset",
      "workspace-live-sync",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.presets, ["workspace-live-sync"])
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["workspace-live-sync-matrix"])
    assert.deepEqual(report.checks.matrices.requiredMatrixClassifications, [
      "kernel-authority",
      "relay-target-freshness",
      "workspace-live-sync-conflict",
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate rejects unknown presets", async () => {
  await assert.rejects(
    execFile(process.execPath, [
      scriptPath,
      "--preset",
      "workspace-live-synch",
      "--json",
    ]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /unknown validation gate preset: workspace-live-synch/)
      return true
    },
  )
})

test("drill validation gate requires matrix name coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--matrix-report",
        reportPath,
        "--require-matrix",
        "cli-matrix,remote-runtime",
        "--require-matrix=hosted-cloud",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.requiredMatrices, ["cli-matrix", "hosted-cloud", "remote-runtime"])
        assert.deepEqual(report.checks.matrices.missingMatrices, ["hosted-cloud", "remote-runtime"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate requires matrix classification coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeClassifiedMatrixReport(reportPath)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--matrix-report",
        reportPath,
        "--require-matrix-classification",
        "kernel-authority,remote-extension-sync",
        "--require-matrix-classification=relay-target-freshness",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.requiredMatrixClassifications, [
          "kernel-authority",
          "relay-target-freshness",
          "remote-extension-sync",
        ])
        assert.deepEqual(report.checks.matrices.missingMatrixClassifications, ["remote-extension-sync"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate rejects unknown matrix classification requirements", async () => {
  await assert.rejects(
    execFile(process.execPath, [
      scriptPath,
      "--require-matrix-classification",
      "remote-extension-synch",
      "--json",
    ]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /unknown required matrix classification: remote-extension-synch/)
      return true
    },
  )
})

test("drill validation gate requires deployment preset coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, { deploymentPresets: "local,self-hosted-relay" })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--matrix-report",
        reportPath,
        "--require-deployment-preset",
        "local,hosted-cloud",
        "--require-deployment-preset=self-hosted-relay",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.requiredDeploymentPresets, ["hosted-cloud", "local", "self-hosted-relay"])
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate requires provider coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, { providers: "codex,opencode" })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--matrix-report",
        reportPath,
        "--require-provider",
        "codex,claude",
        "--require-provider=opencode",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.requiredProviders, ["claude", "codex", "opencode"])
        assert.deepEqual(report.checks.matrices.missingProviders, ["claude"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate requires scenario coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--matrix-report",
        reportPath,
        "--require-scenario",
        "local,remote-collab",
        "--require-scenario=hetzner-collab",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.requiredScenarios, ["hetzner-collab", "local", "remote-collab"])
        assert.deepEqual(report.checks.matrices.missingScenarios, ["hetzner-collab", "remote-collab"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate rejects unknown deployment preset requirements", async () => {
  await assert.rejects(
    execFile(process.execPath, [
      scriptPath,
      "--require-deployment-preset",
      "hosted-clouds",
      "--json",
    ]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /unknown required deployment preset: hosted-clouds/)
      return true
    },
  )
})

test("drill validation gate accepts artifact roots", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-root",
      rootDir,
      "--json",
    ])
    const report = JSON.parse(stdout)

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.equal(report.checks.artifacts.aggregate.totals.artifacts, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate rejects empty configuration", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--json"]),
    (error) => {
      const report = JSON.parse(error.stdout)
      assert.equal(error.code, 1)
      assert.equal(report.status, "failed")
      assert.equal(report.checks.configuration.error, "no validation checks configured")
      return true
    },
  )
})

test("drill validation gate exits non-zero for preserved failures", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-cli-"))
  try {
    const failureRoot = path.join(rootDir, "failed")
    await writeFailureManifest(path.join(failureRoot, "arroba-drill-failure.json"))

    await assert.rejects(
      execFile(process.execPath, [scriptPath, "--failure-manifest", failureRoot, "--json"]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.deepEqual(report.checks.failures.inputs, [failureRoot])
        assert.deepEqual(report.checks.failures.manifestPaths, [path.join(failureRoot, "arroba-drill-failure.json")])
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

async function writeMatrixReport(file, metadata = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix: "cli-matrix",
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios: [{
      id: "local",
      description: "local scenario",
      requires: [],
      exitCriteria: [],
      status: "passed",
      expectedFailure: false,
      classification: null,
      durationMs: 10,
      reason: null,
      command: "node",
      args: ["local.mjs"],
      artifactHints: [],
    }],
  }, null, 2)}\n`, "utf8")
}

async function writeClassifiedMatrixReport(file) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix: "cli-matrix",
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios: [
      scenario("kernel-authority", "kernel-authority"),
      scenario("relay-target-freshness", "relay-target-freshness"),
    ],
  }, null, 2)}\n`, "utf8")
}

async function writeWorkspaceLiveSyncPresetReport(file) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix: "workspace-live-sync-matrix",
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios: [
      scenario("managed", "workspace-live-sync-conflict"),
      scenario("permission", "kernel-authority"),
      scenario("restart", "relay-target-freshness"),
    ],
  }, null, 2)}\n`, "utf8")
}

function scenario(id, classification) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status: "passed",
    expectedFailure: false,
    classification,
    durationMs: 10,
    reason: null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
  }
}

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
