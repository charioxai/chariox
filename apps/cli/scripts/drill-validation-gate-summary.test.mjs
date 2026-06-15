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
import { runDrillValidationGate } from "./lib/drill-validation-gate.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"
import { workspaceLiveSyncRequiredScenarioDescriptors } from "./lib/workspace-live-sync-fixtures.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-validation-gate-summary.mjs", import.meta.url))

test("drill validation gate summary aggregates discovered reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const passedReportPath = path.join(rootDir, "reports", "passed.json")
    const failedReportPath = path.join(rootDir, "reports", "failed.json")
    const workspaceReportPath = path.join(rootDir, "reports", "workspace-live-sync.json")
    await writeGateReport(passedReportPath, await passingGateReport(rootDir))
    await writeGateReport(failedReportPath, await runDrillValidationGate())
    await writeGateReport(workspaceReportPath, await passingWorkspaceLiveSyncGateReport(rootDir))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-root",
        path.join(rootDir, "reports"),
        "--json",
        "--output",
        outputPath,
        "--output-artifact-index",
        artifactIndexPath,
      ]),
      (error) => {
        const stdoutAggregate = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(stdoutAggregate.status, "failed")
        assert.deepEqual(stdoutAggregate.totals, { reports: 3, passed: 2, failed: 1 })
        assert.deepEqual(stdoutAggregate.reports.map((report) => report.source), [
          failedReportPath,
          passedReportPath,
          workspaceReportPath,
        ])
        return true
      },
    )

    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    assert.equal(fileAggregate.status, "failed")
    assert.deepEqual(fileAggregate.totals, { reports: 3, passed: 2, failed: 1 })
    assert.equal(artifactIndex.metadata.drill, "validation-gate-summary")
    assert.equal(artifactIndex.metadata.status, "failed")
    assert.equal(artifactIndex.metadata.runtimeSignals, "relay-target-freshness,session-authority,workspace-live-sync-state")
    assert.equal(artifactIndex.metadata.runtimeSignalOwners, "kernel-authority,runtime-network,runtime-state")
    assert.match(artifactIndex.metadata.owners, /validation-harness/)
    assert.match(artifactIndex.metadata.classifications, /validation-gate/)
    assert.match(artifactIndex.metadata.classifications, /workspace-live-sync-conflict/)
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "aggregate.json",
      schema: "arroba.drill.validation_gate.aggregate.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("drill validation gate summary help lists artifact coverage requirements", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--help"])

  assert.match(stdout, /--artifact-index PATH/)
  assert.match(stdout, /--artifact-root ROOT/)
  assert.match(stdout, /--require-artifact-coverage-area AREA/)
  assert.match(stdout, /--require-artifact-generated-evidence-kind KIND/)
  assert.match(stdout, /--require-artifact-generated-matrix-limitation KIND/)
  assert.match(stdout, /--require-artifact-runtime-signal ID/)
  assert.match(stdout, /--require-artifact-runtime-signal-owner OWNER/)
  assert.match(stdout, /--require-artifact-owner OWNER/)
  assert.match(stdout, /--require-artifact-classification CLASSIFICATION/)
  assert.match(stdout, /--require-generated-matrix-limitation KIND/)
})

test("drill validation gate summary accepts explicit report paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--json",
    ])
    const aggregate = JSON.parse(stdout)

    assert.equal(aggregate.status, "passed")
    assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary gates generated evidence kinds", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, {
      ...(await passingGateReport(rootDir)),
      generatedEvidence: generatedEvidence(),
    })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--require-generated-evidence-kind",
      "matrix-report,validation-suite-run",
      "--require-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
    ])
    const aggregate = JSON.parse(stdout)
    assert.equal(aggregate.status, "passed")
    assert.deepEqual(aggregate.requiredGeneratedEvidenceKinds, ["matrix-report", "validation-suite-run"])
    assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, [])
    assert.deepEqual(aggregate.requiredGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(aggregate.missingGeneratedMatrixLimitations, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-report",
        reportPath,
        "--require-generated-evidence-kind",
        "not-generated",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /unknown required generated evidence kind: not-generated/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary gates aggregate preset coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "workspace-live-sync.json")
    await writeGateReport(reportPath, await passingWorkspaceLiveSyncGateReport(rootDir))

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--require-preset",
      "workspace-live-sync",
      "--require-platform-coverage-area",
      "matrix-validation",
      "--require-runtime-signal",
      "workspace-live-sync-state",
      "--require-failure-classification",
      "kernel-authority",
      "--require-matrix",
      "workspace-live-sync-matrix",
      "--require-matrix-classification",
      "workspace-live-sync-conflict",
      "--require-matrix-runtime-signal",
      "workspace-live-sync-state",
      "--require-deployment-preset",
      "local",
      "--require-provider",
      "codex",
      "--require-scenario",
      "local-managed-codex",
      "--json",
    ])
    const passedAggregate = JSON.parse(stdout)
    assert.equal(passedAggregate.status, "passed")
    assert.deepEqual(passedAggregate.requiredPresets, ["workspace-live-sync"])
    assert.deepEqual(passedAggregate.missingPresets, [])
    assert.deepEqual(passedAggregate.requiredRuntimeSignals, ["workspace-live-sync-state"])
    assert.deepEqual(passedAggregate.missingRuntimeSignals, [])
    assert.deepEqual(passedAggregate.requiredMatrixRuntimeSignals, ["workspace-live-sync-state"])
    assert.deepEqual(passedAggregate.missingMatrixRuntimeSignals, [])
    assert.equal(
      passedAggregate.matrixRuntimeSignalSources["workspace-live-sync-state"].some((entry) => entry.id === "local-managed-codex"),
      true,
    )
    assert.deepEqual(passedAggregate.missingProviders, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-report",
        reportPath,
        "--require-preset",
        "remote-home-extension",
        "--require-provider",
        "claude",
        "--json",
      ]),
      (error) => {
        const failedAggregate = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(failedAggregate.status, "failed")
        assert.deepEqual(failedAggregate.requiredPresets, ["remote-home-extension"])
        assert.deepEqual(failedAggregate.missingPresets, ["remote-home-extension"])
        assert.deepEqual(failedAggregate.requiredProviders, ["claude"])
        assert.deepEqual(failedAggregate.missingProviders, ["claude"])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary gates aggregate artifact schema coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-report",
        reportPath,
        "--require-artifact-schema",
        "arroba.drill.validation_suite_run.v1",
        "--json",
      ]),
      (error) => {
        const aggregate = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(aggregate.status, "failed")
        assert.deepEqual(aggregate.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(aggregate.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.equal(
          aggregate.nextActions.some((action) =>
            action.nextAction === "run an executable validation suite with --run-json --output PATH --output-artifact-index PATH, then rerun the validation gate aggregate"),
          true,
        )
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary gates aggregate artifact coverage areas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-report",
        reportPath,
        "--require-artifact-coverage-area",
        "distributed-observability",
        "--json",
      ]),
      (error) => {
        const aggregate = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(aggregate.status, "failed")
        assert.deepEqual(aggregate.requiredArtifactCoverageAreas, ["distributed-observability"])
        assert.deepEqual(aggregate.missingArtifactCoverageAreas, ["distributed-observability"])
        assert.equal(
          aggregate.nextActions.some((action) =>
            action.nextAction === "provide validation gate reports with artifact coverage areas: distributed-observability"),
          true,
        )
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary gates artifact generated matrix limitation coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-report",
        reportPath,
        "--require-artifact-generated-matrix-limitation",
        "dry-run-classification-coverage",
        "--json",
      ]),
      (error) => {
        const aggregate = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(aggregate.status, "failed")
        assert.deepEqual(aggregate.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
        assert.deepEqual(aggregate.missingArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
        assert.equal(
          aggregate.nextActions.some((action) =>
            action.nextAction === "provide validation gate artifact indexes with generated matrix limitations: dry-run-classification-coverage"),
          true,
        )
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary consumes artifact metadata inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    const evidenceRoot = path.join(rootDir, "evidence", "distributed-gate")
    const evidencePath = path.join(evidenceRoot, "distributed-runtime-gate.json")
    await mkdir(evidenceRoot, { recursive: true })
    await writeFile(evidencePath, `${JSON.stringify(await passingGateReport(rootDir), null, 2)}\n`, "utf8")
    await writeDrillArtifactIndex({
      rootDir: evidenceRoot,
      artifacts: ["distributed-runtime-gate.json"],
      metadata: {
        exitCriterionStatuses: "dry-run",
        incompleteExitCriterionStatuses: "dry-run",
        generatedMatrixLimitations: "dry-run-classification-coverage",
      },
    })

    const outputPath = path.join(rootDir, "aggregate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--artifact-root",
      path.join(rootDir, "evidence"),
      "--require-artifact-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutAggregate = JSON.parse(stdout)
    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileAggregate, stdoutAggregate)
    assert.equal(stdoutAggregate.status, "passed")
    assert.deepEqual(stdoutAggregate.totals, { reports: 1, passed: 1, failed: 0 })
    assert.deepEqual(stdoutAggregate.artifactCoverageInputs.map((input) => input.status), ["passed"])
    assert.deepEqual(stdoutAggregate.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(stdoutAggregate.missingArtifactGeneratedMatrixLimitations, [])
    assert.equal(stdoutAggregate.coverage.artifactExitCriterionStatuses["dry-run"], 1)
    assert.equal(stdoutAggregate.coverage.artifactIncompleteExitCriterionStatuses["dry-run"], 1)
    assert.equal(stdoutAggregate.coverage.artifactGeneratedMatrixLimitations["dry-run-classification-coverage"], 1)
    assert.deepEqual(stdoutAggregate.reports.map((report) => report.source), [reportPath])
    assert.deepEqual(stdoutAggregate.artifactCoverageInputs.map((input) => input.source), ["artifact metadata inputs"])
    assert.equal(artifactIndex.metadata.artifactCoverageInputCount, "1")
    assert.equal(artifactIndex.metadata.artifactCoverageInputSources, "artifact metadata inputs")
    assert.equal(artifactIndex.metadata.exitCriterionStatuses, "dry-run")
    assert.equal(artifactIndex.metadata.incompleteExitCriterionStatuses, "dry-run")
    assert.equal(artifactIndex.metadata.generatedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixLimitations, "dry-run-classification-coverage")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary fails stale artifact metadata inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const evidenceRoot = path.join(rootDir, "evidence")
    const evidencePath = path.join(evidenceRoot, "distributed-runtime-gate.json")
    await mkdir(evidenceRoot, { recursive: true })
    await writeGateReport(evidencePath, await passingGateReport(rootDir))
    await writeDrillArtifactIndex({
      rootDir: evidenceRoot,
      artifacts: ["distributed-runtime-gate.json"],
    })
    const evidenceIndexPath = path.join(evidenceRoot, "arroba-drill-artifacts.json")
    await rewriteArtifactIndexCreatedAt(evidenceIndexPath, new Date(Date.now() - 500).toISOString())

    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-report",
        reportPath,
        "--artifact-index",
        evidenceIndexPath,
        "--require-artifact-max-age-ms",
        "100",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const aggregate = JSON.parse(error.stdout)
        assert.equal(aggregate.status, "failed")
        assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
        assert.equal(aggregate.artifactCoverageInputs.length, 1)
        assert.equal(aggregate.artifactCoverageInputs[0].status, "failed")
        assert.equal(aggregate.artifactCoverageInputs[0].checks.artifacts, "failed")
        assert.equal(
          aggregate.nextActions.some(({ classification }) => classification === "artifact-staleness"),
          true,
        )
        return true
      },
    )

    const fresh = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--artifact-index",
      evidenceIndexPath,
      "--require-artifact-max-age-ms",
      "3600000",
      "--json",
    ])).stdout)
    assert.equal(fresh.status, "passed")
    assert.equal(fresh.artifactCoverageInputs[0].status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary indexes aggregate artifact coverage metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const evidenceRoot = path.join(rootDir, "evidence")
    const evidencePath = path.join(evidenceRoot, "validation-suite.json")
    const evidenceIndexPath = path.join(evidenceRoot, "arroba-drill-artifacts.json")
    await mkdir(evidenceRoot, { recursive: true })
    await writeFile(evidencePath, `${JSON.stringify(validationSuiteRunArtifact(), null, 2)}\n`, "utf8")
    await writeDrillArtifactIndex({
      rootDir: evidenceRoot,
      artifacts: ["validation-suite.json"],
      metadata: { coverageAreas: "distributed-observability" },
    })

    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await runDrillValidationGate({
      artifactIndexes: [evidenceIndexPath],
      requiredArtifactCoverageAreas: ["distributed-observability"],
    }))

    const outputPath = path.join(rootDir, "aggregate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--require-artifact-coverage-area",
      "distributed-observability",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])

    const aggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    assert.equal(aggregate.status, "passed")
    assert.deepEqual(aggregate.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(aggregate.missingArtifactCoverageAreas, [])
    assert.equal(artifactIndex.metadata.coverageAreas, "distributed-observability")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary rejects empty inputs", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /no validation gate reports found/)
      return true
    },
  )
})

async function passingGateReport(rootDir) {
  const bundleDir = path.join(rootDir, `bundle-${Date.now()}-${Math.random().toString(16).slice(2)}`)
  await writeDrillPlatformBundle(bundleDir)
  return runDrillValidationGate({ platformBundleDir: bundleDir })
}

function generatedEvidence() {
  return {
    validationSuites: {
      enabled: true,
      artifactIndexes: ["/tmp/generated-validation-suite/arroba-drill-artifacts.json"],
      outputRoots: ["/tmp/generated-validation-suite"],
    },
    matrixReports: {
      enabled: true,
      roots: ["/tmp/generated-matrix"],
      dryRun: false,
      continueOnFailure: true,
      limitations: [{
        kind: "dry-run-classification-coverage",
        owner: "validation-harness",
        nextAction: "rerun generated matrix reports without --dry-run before release",
      }],
      commands: [{
        artifactIndexPath: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
        args: ["--include-remote"],
        cwd: "/tmp/arroba",
        reportPath: "/tmp/generated-matrix/workspace-live-sync-matrix.json",
        scriptPath: "/tmp/arroba/apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs",
      }],
    },
  }
}

function validationSuiteRunArtifact() {
  const manifest = {
    schema: "arroba.drill.validation_suite.v1",
    command: "node --test apps/cli/scripts/drill-validation-gate-summary.test.mjs",
    testCount: 1,
    testPaths: ["apps/cli/scripts/drill-validation-gate-summary.test.mjs"],
  }
  return {
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-01-01T00:00:00.000Z",
    completedAt: "2026-01-01T00:00:00.500Z",
    durationMs: 500,
    exitCode: 0,
    signal: null,
    error: null,
    command: manifest.command,
    testCount: manifest.testCount,
    testPaths: manifest.testPaths,
    manifest,
  }
}

async function passingWorkspaceLiveSyncGateReport(rootDir) {
  const bundleDir = path.join(rootDir, `bundle-${Date.now()}-${Math.random().toString(16).slice(2)}`)
  const matrixPath = path.join(rootDir, `matrix-${Date.now()}-${Math.random().toString(16).slice(2)}.json`)
  await writeDrillPlatformBundle(bundleDir)
  await writeGateReport(matrixPath, {
    schema: "arroba.drill.matrix.v1",
    matrix: "workspace-live-sync-matrix",
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "codex,opencode",
    },
    scenarios: workspaceLiveSyncRequiredScenarios(),
  })
  return runDrillValidationGate({
    platformBundleDir: bundleDir,
    matrixReports: [matrixPath],
    presets: ["workspace-live-sync"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local-managed-codex"],
  })
}

function workspaceLiveSyncRequiredScenarios() {
  return workspaceLiveSyncRequiredScenarioDescriptors().map(({ id, classification, runtimeSignals }) => {
    return passingScenario(id, classification, runtimeSignals)
  })
}

function passingScenario(id, classification, runtimeSignals = []) {
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
    runtimeSignals,
  }
}

async function writeGateReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`, "utf8")
}

async function rewriteArtifactIndexCreatedAt(indexPath, createdAt) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  index.createdAt = createdAt
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
}
