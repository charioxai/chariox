import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  describeDrillValidationGatePresets,
  drillValidationGateExitCode,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  runDrillValidationGate,
  summarizeDrillValidationGateReports,
  validateDrillValidationGateAggregate,
  validateDrillValidationGateReport,
} from "./drill-validation-gate.mjs"
import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
import { writeDrillPlatformBundle } from "./drill-platform-bundle.mjs"

test("describes validation gate presets", () => {
  const presets = describeDrillValidationGatePresets()
  assert.deepEqual(presets.map((preset) => preset.name), ["remote-home-extension", "workspace-live-sync"])
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["workspace-live-sync"] })[0].requiredMatrixClassifications,
    ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
  )
  assert.throws(
    () => describeDrillValidationGatePresets({ names: ["workspace-live-synch"] }),
    /unknown validation gate preset: workspace-live-synch/,
  )
})

test("passes with valid platform bundle and complete matrix reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const matrixRoot = path.join(rootDir, "matrices")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(path.join(matrixRoot, "matrix.json"), matrixReport())

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixRoots: [matrixRoot],
      requireComplete: true,
    })

    assert.equal(report.schema, "arroba.drill.validation_gate.v1")
    assert.equal(report.status, "passed")
    assert.equal(drillValidationGateExitCode(report), 0)
    assert.equal(report.checks.configuration.status, "passed")
    assert.equal(report.checks.platformBundle.status, "passed")
    assert.deepEqual(report.checks.platformBundle.validationSuite, {
      testCount: 29,
      coverageAreas: [
        { id: "artifact-contracts", testCount: 7 },
        { id: "failure-diagnostics", testCount: 3 },
        { id: "matrix-validation", testCount: 10 },
        { id: "runtime-fixtures", testCount: 7 },
        { id: "suite-contract", testCount: 2 },
      ],
    })
    assert.equal(report.checks.platformBundle.failureTaxonomy.drill.includes("kernel-authority"), true)
    assert.equal(report.checks.platformBundle.failureTaxonomy.scenario.includes("remote-extension-sync"), true)
    assert.equal(report.checks.matrices.status, "passed")
    assert.equal(report.checks.failures.status, "skipped")
    assert.deepEqual(report.nextActions, [])
    assert.doesNotThrow(() => validateDrillValidationGateReport(report))
    assert.match(formatDrillValidationGateSummary(report), /status=passed/)
    assert.match(formatDrillValidationGateSummary(report), /platform_validation_suite_tests=29 coverage=artifact-contracts:7/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates platform bundle validation suite coverage areas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const pass = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredPlatformCoverageAreas: ["runtime-fixtures,matrix-validation"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.platformBundle.requiredCoverageAreas, ["matrix-validation", "runtime-fixtures"])
    assert.deepEqual(pass.checks.platformBundle.missingCoverageAreas, [])
    assert.match(formatDrillValidationGateSummary(pass), /platform_required_coverage_areas=matrix-validation,runtime-fixtures missing=none/)

    const fail = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredPlatformCoverageAreas: ["runtime-fixtures", "hosted-cloud-drills"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.platformBundle.missingCoverageAreas, ["hosted-cloud-drills"])
    assert.match(fail.checks.platformBundle.error, /missing platform coverage areas: hosted-cloud-drills/)
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "provide a drill platform bundle covering: hosted-cloud-drills",
      },
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
      },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates platform bundle failure taxonomy classifications", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const pass = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredFailureClassifications: ["kernel-authority,remote-extension-sync", "workspace-live-sync-conflict"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.platformBundle.requiredFailureClassifications, [
      "kernel-authority",
      "remote-extension-sync",
      "workspace-live-sync-conflict",
    ])
    assert.deepEqual(pass.checks.platformBundle.missingFailureClassifications, [])
    assert.match(
      formatDrillValidationGateSummary(pass),
      /platform_required_failure_classifications=kernel-authority,remote-extension-sync,workspace-live-sync-conflict missing=none/,
    )
    assert.match(formatDrillValidationGateSummary(pass), /platform_failure_taxonomy=drill:\d+ scenario:\d+/)

    await assert.rejects(
      () => runDrillValidationGate({
        platformBundleDir: bundleDir,
        requiredFailureClassifications: ["kernel-authority", "kernel-authorities"],
      }),
      /unknown required failure classification: kernel-authorities/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("applies validation gate requirement presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "workspace-live-sync.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(reportPath, matrixReport({
      matrix: "workspace-live-sync-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "codex,opencode",
      },
      scenarios: [
        scenario("managed", "passed", { classification: "workspace-live-sync-conflict" }),
        scenario("permission", "passed", { classification: "kernel-authority" }),
        scenario("restart", "passed", { classification: "relay-target-freshness" }),
      ],
    }))

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixReports: [reportPath],
      presets: ["workspace-live-sync"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.presets, ["workspace-live-sync"])
    assert.deepEqual(report.checks.platformBundle.requiredFailureClassifications, [
      "kernel-authority",
      "relay-target-freshness",
      "workspace-live-sync-conflict",
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["workspace-live-sync-matrix"])
    assert.deepEqual(report.checks.matrices.requiredMatrixClassifications, [
      "kernel-authority",
      "relay-target-freshness",
      "workspace-live-sync-conflict",
    ])
    assert.deepEqual(report.checks.matrices.requiredDeploymentPresets, ["local"])
    assert.deepEqual(report.checks.matrices.requiredProviders, ["codex"])
    assert.deepEqual(report.checks.matrices.requiredScenarios, ["managed"])
    assert.match(formatDrillValidationGateSummary(report), /presets=workspace-live-sync/)
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_classifications=kernel-authority,relay-target-freshness,workspace-live-sync-conflict missing=none/)
    const aggregate = summarizeDrillValidationGateReports([report], { sources: ["workspace-live-sync.json"] })
    assert.deepEqual(aggregate.coverage.presets, { "workspace-live-sync": 1 })
    assert.deepEqual(aggregate.reports[0].presets, ["workspace-live-sync"])
    assert.match(formatDrillValidationGateAggregateSummary(aggregate), /presets: workspace-live-sync=1/)
    const requiredAggregate = summarizeDrillValidationGateReports([report], {
      sources: ["workspace-live-sync.json"],
      requiredPresets: ["workspace-live-sync"],
      requiredPlatformCoverageAreas: ["matrix-validation"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
    })
    assert.equal(requiredAggregate.status, "passed")
    assert.deepEqual(requiredAggregate.requiredPresets, ["workspace-live-sync"])
    assert.deepEqual(requiredAggregate.missingPresets, [])
    assert.deepEqual(requiredAggregate.missingPlatformCoverageAreas, [])
    assert.deepEqual(requiredAggregate.missingFailureClassifications, [])
    assert.deepEqual(requiredAggregate.missingMatrices, [])
    assert.deepEqual(requiredAggregate.missingMatrixClassifications, [])
    assert.deepEqual(requiredAggregate.missingDeploymentPresets, [])
    assert.deepEqual(requiredAggregate.missingProviders, [])
    assert.deepEqual(requiredAggregate.missingScenarios, [])
    assert.match(formatDrillValidationGateAggregateSummary(requiredAggregate), /required_presets=workspace-live-sync missing=none/)
    assert.match(formatDrillValidationGateAggregateSummary(requiredAggregate), /required_providers=codex missing=none/)
    const missingAggregate = summarizeDrillValidationGateReports([report], {
      sources: ["workspace-live-sync.json"],
      requiredPresets: ["remote-home-extension"],
      requiredProviders: ["claude"],
    })
    assert.equal(missingAggregate.status, "failed")
    assert.deepEqual(missingAggregate.requiredPresets, ["remote-home-extension"])
    assert.deepEqual(missingAggregate.missingPresets, ["remote-home-extension"])
    assert.deepEqual(missingAggregate.requiredProviders, ["claude"])
    assert.deepEqual(missingAggregate.missingProviders, ["claude"])
    assert.deepEqual(missingAggregate.nextActions.map(({ classification, nextAction }) => ({ classification, nextAction })), [{
      classification: "matrix-coverage",
      nextAction: "provide validation gate reports requiring providers: claude",
    }, {
      classification: "validation-gate",
      nextAction: "provide validation gate reports for presets: remote-home-extension",
    }])
    assert.match(formatDrillValidationGateAggregateSummary(missingAggregate), /required_presets=remote-home-extension missing=remote-home-extension/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unknown validation gate presets", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      presets: ["workspace-live-synch"],
    }),
    /unknown validation gate preset: workspace-live-synch/,
  )
})

test("fails platform coverage requirements without a platform bundle", async () => {
  const report = await runDrillValidationGate({
    requiredPlatformCoverageAreas: ["runtime-fixtures"],
  })

  assert.equal(report.status, "failed")
  assert.equal(report.checks.platformBundle.status, "failed")
  assert.equal(report.checks.platformBundle.error, "no platform bundle provided")
  assert.deepEqual(report.checks.platformBundle.missingCoverageAreas, ["runtime-fixtures"])
})

test("fails platform failure classification requirements without a platform bundle", async () => {
  const report = await runDrillValidationGate({
    requiredFailureClassifications: ["kernel-authority", "remote-extension-sync"],
  })

  assert.equal(report.status, "failed")
  assert.equal(report.checks.platformBundle.status, "failed")
  assert.equal(report.checks.platformBundle.error, "no platform bundle provided")
  assert.deepEqual(report.checks.platformBundle.missingFailureClassifications, ["kernel-authority", "remote-extension-sync"])
  assert.deepEqual(report.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "provide a drill platform bundle covering failure classifications: kernel-authority, remote-extension-sync",
    },
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
    },
  ])
})

test("rejects unknown required failure classifications", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      requiredFailureClassifications: ["kernel-authority", "remote-extension-synch"],
    }),
    /unknown required failure classification: remote-extension-synch/,
  )
})


test("passes with explicit matrix report paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport())

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requireComplete: true,
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.inputs, [reportPath])
    assert.deepEqual(report.checks.matrices.reportPaths, [reportPath])
    assert.match(formatDrillValidationGateSummary(report), /matrices=passed roots=0 inputs=1 reports=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required matrix name coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const localReport = path.join(rootDir, "local.json")
    const remoteReport = path.join(rootDir, "remote.json")
    await writeMatrixReport(localReport, matrixReport({ matrix: "local-runtime" }))
    await writeMatrixReport(remoteReport, matrixReport({ matrix: "remote-runtime" }))

    const pass = await runDrillValidationGate({
      matrixReports: [localReport, remoteReport],
      requiredMatrices: ["local-runtime,remote-runtime"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredMatrices, ["local-runtime", "remote-runtime"])
    assert.deepEqual(pass.checks.matrices.missingMatrices, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_names=local-runtime,remote-runtime missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [localReport],
      requiredMatrices: ["hosted-cloud", "local-runtime"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingMatrices, ["hosted-cloud"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run missing drill matrices: hosted-cloud",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required classification coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      scenarios: [
        scenario("kernel-authority", "passed", { classification: "kernel-authority" }),
        scenario("relay-freshness", "passed", { classification: "relay-target-freshness" }),
      ],
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredMatrixClassifications: ["kernel-authority,relay-target-freshness"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredMatrixClassifications, ["kernel-authority", "relay-target-freshness"])
    assert.deepEqual(pass.checks.matrices.missingMatrixClassifications, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_classifications=kernel-authority,relay-target-freshness missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredMatrixClassifications: ["kernel-authority", "remote-extension-sync"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingMatrixClassifications, ["remote-extension-sync"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports covering failure classifications: remote-extension-sync",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unknown required matrix classifications", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      requiredMatrixClassifications: ["kernel-authority", "remote-extension-synch"],
    }),
    /unknown required matrix classification: remote-extension-synch/,
  )
})

test("passes when matrix reports cover required deployment presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: { deploymentPresets: "hosted-cloud,local,self-hosted-relay" },
    }))

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredDeploymentPresets: ["self-hosted-relay,local", "hosted-cloud"],
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.requiredDeploymentPresets, ["hosted-cloud", "local", "self-hosted-relay"])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_deployment_presets=hosted-cloud,local,self-hosted-relay missing=none/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when matrix reports miss required deployment presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: { deploymentPresets: "local,self-hosted-relay" },
    }))

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredDeploymentPresets: ["local", "hosted-cloud", "hetzner"],
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.status, "failed")
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hetzner", "hosted-cloud"])
    assert.deepEqual(report.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing deployment presets: hetzner, hosted-cloud",
    }])
    assert.match(formatDrillValidationGateSummary(report), /missing=hetzner,hosted-cloud/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required provider coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: { providers: "codex,opencode" },
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredProviders: ["codex,opencode"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredProviders, ["codex", "opencode"])
    assert.deepEqual(pass.checks.matrices.missingProviders, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_providers=codex,opencode missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredProviders: ["claude", "codex"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingProviders, ["claude"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing providers: claude",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required scenario coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      scenarios: [
        scenario("local-single-user", "passed"),
        scenario("remote-collab", "passed"),
      ],
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredScenarios: ["local-single-user,remote-collab"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredScenarios, ["local-single-user", "remote-collab"])
    assert.deepEqual(pass.checks.matrices.missingScenarios, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_scenarios=local-single-user,remote-collab missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredScenarios: ["hetzner-collab", "local-single-user"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingScenarios, ["hetzner-collab"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing scenarios: hetzner-collab",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unknown required deployment presets", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      requiredDeploymentPresets: ["local", "hosted-clouds"],
    }),
    /unknown required deployment preset: hosted-clouds/,
  )
})

test("fails when no validation checks are configured", async () => {
  const report = await runDrillValidationGate()

  assert.equal(report.status, "failed")
  assert.equal(report.checks.configuration.error, "no validation checks configured")
  assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
    { owner: "validation-harness", classification: "validation-gate" },
  ])
  assert.match(formatDrillValidationGateSummary(report), /configuration=failed/)
})

test("fails when configured matrix roots contain no reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const report = await runDrillValidationGate({ matrixRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.error, "no matrix reports found")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "matrix-artifacts" },
    ])
    assert.equal(drillValidationGateExitCode(report), 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when require-complete sees dry-run matrix scenarios", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    await writeMatrixReport(path.join(rootDir, "matrix.json"), matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("remote", "dry-run")],
    }))

    const report = await runDrillValidationGate({
      matrixRoots: [rootDir],
      requireComplete: true,
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.aggregate.status, "dry-run")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "incomplete-matrix" },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when preserved failure manifests are found", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    await writeFailureManifest(path.join(rootDir, "failed", "arroba-drill-failure.json"))

    const report = await runDrillValidationGate({ failureRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.failures.aggregate.total, 1)
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "provider-account", classification: "provider-auth" },
    ])
    assert.match(formatDrillValidationGateSummary(report), /failure_total=1/)
    assert.match(formatDrillValidationGateSummary(report), /next actions:/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails with explicit failure manifest paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const manifestPath = path.join(rootDir, "arroba-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const report = await runDrillValidationGate({ failureInputs: [manifestPath] })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.checks.failures.inputs, [manifestPath])
    assert.deepEqual(report.checks.failures.manifestPaths, [manifestPath])
    assert.match(formatDrillValidationGateSummary(report), /failures=failed roots=0 inputs=1 manifests=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("passes with explicit artifact index paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    const indexPath = path.join(rootDir, "arroba-drill-artifacts.json")

    const report = await runDrillValidationGate({ artifactIndexes: [indexPath] })

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, [indexPath])
    assert.deepEqual(report.checks.artifacts.indexPaths, [indexPath])
    assert.equal(report.checks.artifacts.aggregate.totals.artifacts, 1)
    assert.match(formatDrillValidationGateSummary(report), /artifacts=passed roots=0 inputs=1 indexes=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when configured artifact roots contain no indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const report = await runDrillValidationGate({ artifactRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.artifacts.error, "no artifact indexes found")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "artifact-index" },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when artifact indexes point at tampered artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    await writeFile(reportPath, "{\"schema\":\"tampered\"}\n", "utf8")

    const report = await runDrillValidationGate({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.artifacts.status, "failed")
    assert.match(report.checks.artifacts.error, /sha256 mismatch/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation gate reports with mismatched top-level status", async () => {
  const report = await runDrillValidationGate()

  assert.throws(
    () => validateDrillValidationGateReport({ ...report, status: "passed" }),
    /status does not match check statuses/,
  )
})

test("rejects malformed platform bundle artifact evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    const malformed = {
      ...report,
      checks: {
        ...report.checks,
        platformBundle: {
          ...report.checks.platformBundle,
          artifacts: [{
            ...report.checks.platformBundle.artifacts[0],
            sha256: "not-a-sha",
          }],
        },
      },
    }

    assert.throws(
      () => formatDrillValidationGateSummary(malformed),
      /artifacts\[0\] has invalid sha256/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects inconsistent platform bundle validation suite evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    const malformed = {
      ...report,
      checks: {
        ...report.checks,
        platformBundle: {
          ...report.checks.platformBundle,
          validationSuite: {
            ...report.checks.platformBundle.validationSuite,
            coverageAreas: report.checks.platformBundle.validationSuite.coverageAreas.slice(1),
          },
        },
      },
    }

    assert.throws(
      () => formatDrillValidationGateSummary(malformed),
      /coverageAreas do not match testCount/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("resolves explicit failure root inputs to manifest paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const failureRoot = path.join(rootDir, "failed")
    const manifestPath = path.join(failureRoot, "arroba-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const report = await runDrillValidationGate({ failureInputs: [failureRoot] })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.checks.failures.inputs, [failureRoot])
    assert.deepEqual(report.checks.failures.manifestPaths, [manifestPath])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("reads and discovers validation gate report artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8")
    await writeFile(path.join(rootDir, "reports", "unrelated.json"), "{\"schema\":\"other\"}\n", "utf8")

    assert.deepEqual(await findDrillValidationGateReportPaths([rootDir]), [reportPath])
    assert.deepEqual(await readDrillValidationGateReport(reportPath), report)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("summarizes validation gate reports", async () => {
  const passed = await runDrillValidationGate({
    failureRoots: ["/tmp/no-such-arroba-failure-root"],
  })
  const failed = await runDrillValidationGate()
  const aggregate = summarizeDrillValidationGateReports([passed, failed], {
    sources: ["passed.json", "failed.json"],
  })

  assert.equal(aggregate.schema, "arroba.drill.validation_gate.aggregate.v1")
  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.totals, { reports: 2, passed: 1, failed: 1 })
  assert.deepEqual(aggregate.reports.map((report) => report.source), ["passed.json", "failed.json"])
  assert.deepEqual(aggregate.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
    { owner: "validation-harness", classification: "validation-gate" },
  ])
  assert.doesNotThrow(() => validateDrillValidationGateAggregate(aggregate))
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /status=failed reports=2 passed=1 failed=1/)
})

test("summarizes validation gate matrix coverage across reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "codex,opencode",
      },
    }))
    const passed = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredPlatformCoverageAreas: ["runtime-fixtures"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["test-matrix"],
      requiredMatrixClassifications: ["kernel-authority"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["local"],
    })
    const failed = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredPlatformCoverageAreas: ["hosted-cloud-drills"],
      requiredFailureClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
      requiredMatrices: ["hosted-matrix", "test-matrix"],
      requiredMatrixClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
      requiredDeploymentPresets: ["hosted-cloud", "local"],
      requiredProviders: ["claude", "codex"],
      requiredScenarios: ["remote"],
    })
    const aggregate = summarizeDrillValidationGateReports([passed, failed], {
      sources: ["passed.json", "failed.json"],
    })

    assert.equal(aggregate.status, "failed")
    assert.deepEqual(aggregate.coverage, {
      presets: {},
      requiredPlatformCoverageAreas: { "hosted-cloud-drills": 1, "runtime-fixtures": 1 },
      missingPlatformCoverageAreas: { "hosted-cloud-drills": 1, "runtime-fixtures": 1 },
      requiredFailureClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      missingFailureClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      requiredMatrices: { "hosted-matrix": 1, "test-matrix": 2 },
      missingMatrices: { "hosted-matrix": 1 },
      requiredMatrixClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      missingMatrixClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      requiredDeploymentPresets: { "hosted-cloud": 1, local: 2 },
      missingDeploymentPresets: { "hosted-cloud": 1 },
      requiredProviders: { claude: 1, codex: 2 },
      missingProviders: { claude: 1 },
      requiredScenarios: { local: 1, remote: 1 },
      missingScenarios: { remote: 1 },
    })
    assert.deepEqual(aggregate.reports.map((report) => report.platformCoverage), [
      {
        requiredCoverageAreas: ["runtime-fixtures"],
        missingCoverageAreas: ["runtime-fixtures"],
        requiredFailureClassifications: ["kernel-authority"],
        missingFailureClassifications: ["kernel-authority"],
      },
      {
        requiredCoverageAreas: ["hosted-cloud-drills"],
        missingCoverageAreas: ["hosted-cloud-drills"],
        requiredFailureClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
        missingFailureClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
      },
    ])
    assert.deepEqual(aggregate.reports.map((report) => report.matrixCoverage), [
      {
        requiredMatrices: ["test-matrix"],
        missingMatrices: [],
        requiredMatrixClassifications: ["kernel-authority"],
        missingMatrixClassifications: ["kernel-authority"],
        requiredDeploymentPresets: ["local"],
        missingDeploymentPresets: [],
        requiredProviders: ["codex"],
        missingProviders: [],
        requiredScenarios: ["local"],
        missingScenarios: [],
      },
      {
        requiredMatrices: ["hosted-matrix", "test-matrix"],
        missingMatrices: ["hosted-matrix"],
        requiredMatrixClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
        missingMatrixClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
        requiredDeploymentPresets: ["hosted-cloud", "local"],
        missingDeploymentPresets: ["hosted-cloud"],
        requiredProviders: ["claude", "codex"],
        missingProviders: ["claude"],
        requiredScenarios: ["remote"],
        missingScenarios: ["remote"],
      },
    ])
    const text = formatDrillValidationGateAggregateSummary(aggregate)
    assert.match(text, /coverage:/)
    assert.match(text, /missing_platform_coverage_areas: hosted-cloud-drills=1 runtime-fixtures=1/)
    assert.match(text, /required_failure_classifications: kernel-authority=1 remote-extension-sync=1 workspace-live-sync-conflict=1/)
    assert.match(text, /missing_failure_classifications: kernel-authority=1 remote-extension-sync=1 workspace-live-sync-conflict=1/)
    assert.match(text, /missing_matrices: hosted-matrix=1/)
    assert.match(text, /missing_matrix_classifications: kernel-authority=1 remote-extension-sync=1 workspace-live-sync-conflict=1/)
    assert.match(text, /required_deployment_presets: hosted-cloud=1 local=2/)
    assert.match(text, /missing_providers: claude=1/)
    assert.match(text, /missing_scenarios: remote=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("reads and discovers validation gate aggregate artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const aggregatePath = path.join(rootDir, "reports", "aggregate.json")
    const aggregate = summarizeDrillValidationGateReports([await runDrillValidationGate({
      failureRoots: ["/tmp/no-such-arroba-failure-root"],
    })])
    await mkdir(path.dirname(aggregatePath), { recursive: true })
    await writeFile(aggregatePath, `${JSON.stringify(aggregate, null, 2)}\n`, "utf8")
    await writeFile(path.join(rootDir, "reports", "gate.json"), `${JSON.stringify(await runDrillValidationGate(), null, 2)}\n`, "utf8")

    assert.deepEqual(await findDrillValidationGateAggregatePaths([rootDir]), [aggregatePath])
    assert.deepEqual(await readDrillValidationGateAggregate(aggregatePath), aggregate)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects inconsistent validation gate aggregates", async () => {
  const aggregate = summarizeDrillValidationGateReports([await runDrillValidationGate()])

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      totals: {
        ...aggregate.totals,
        failed: 0,
      },
    }),
    /totals do not match reports/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        requiredProviders: { codex: 2 },
      },
    }),
    /coverage does not match reports/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      status: "passed",
      requiredPresets: ["workspace-live-sync"],
      missingPresets: [],
    }),
    /missingPresets does not match reports/,
  )
})

async function writeMatrixReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`, "utf8")
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

function matrixReport(overrides = {}) {
  const scenarios = overrides.scenarios ?? [scenario("local", "passed")]
  const status = overrides.status ?? (scenarios.some((entry) => entry.status === "failed") ? "failed" : "passed")
  const dryRun = overrides.dryRun ?? false
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status,
    dryRun,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios,
    ...overrides,
  }
}

function scenario(id, status, overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status,
    expectedFailure: false,
    classification: status === "failed" ? "child-process" : null,
    durationMs: status === "skipped" || status === "dry-run" ? 0 : 10,
    reason: status === "failed" ? "code=1" : status === "skipped" ? "not run" : null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    ...overrides,
  }
}
