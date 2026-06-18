import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex, writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO } from "./lib/drill-distributed-runtime-evidence.mjs"
import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"
import { drillRuntimeSignalOwnersFor, drillRuntimeSignalsManifest } from "./lib/drill-runtime-signals.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-distributed-runtime-gate.mjs", import.meta.url))
const summaryScriptPath = fileURLToPath(new URL("./drill-validation-gate-summary.mjs", import.meta.url))

test("distributed runtime gate help lists artifact evidence requirements", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--help"])

  assert.match(stdout, /--require-artifact-generated-matrix-name NAME\[,NAME\]/)
  assert.match(stdout, /--require-artifact-generated-matrix-repo REPO\[,REPO\]/)
  assert.match(stdout, /--require-artifact-generated-validation-suite-artifact-index PATH\[,PATH\]/)
  assert.match(stdout, /--require-artifact-validation-preset NAME\[,NAME\]/)
  assert.match(stdout, /--require-artifact-planned-owner OWNER\[,OWNER\]/)
  assert.match(stdout, /--require-artifact-planned-classification KIND\[,KIND\]/)
})

test("distributed runtime gate passes with complete OSS and Cloud matrix evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const outputPath = path.join(rootDir, "gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
      providerAccountAliases: "codex=work",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      providerAccountAliases: "opencode=zen",
      plannedOwners: "validation-platform",
      plannedClassifications: "workspace-live-sync-conflict",
    })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-complete",
      "--require-runtime-signal",
      "slice-auth-state",
      "--require-matrix-runtime-signal",
      "slice-auth-state",
      "--require-artifact-provider-account-alias",
      "codex=work",
      "--require-artifact-provider-account-alias",
      "opencode=zen",
      "--require-artifact-validation-preset",
      "cloud-distributed-runtime",
      "--require-artifact-planned-owner",
      "validation-platform",
      "--require-artifact-planned-classification",
      "workspace-live-sync-conflict",
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
    assert.deepEqual(report.generatedEvidence, {
      matrixReports: {
        artifactIndexes: [],
        commands: [],
        continueOnFailure: false,
        dryRun: false,
        enabled: false,
        limitations: [],
        roots: [],
      },
      validationSuites: {
        artifactIndexes: [],
        commands: [],
        enabled: false,
        failureRoots: [],
        outputRoots: [],
      },
    })
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(report.checks.artifacts.missingArtifactCoverageAreas, [])
    assert.equal(report.checks.artifacts.aggregate.coverageAreas["distributed-observability"], 2)
    assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.deepEqual(report.checks.artifacts.requiredArtifactKinds, ["validation-suite-run"])
    assert.deepEqual(report.checks.artifacts.requiredArtifactEvidenceRepos, ["cloud", "oss"])
    assert.deepEqual(report.checks.artifacts.missingArtifactEvidenceRepos, [])
    assert.deepEqual(report.checks.artifacts.requiredArtifactProviderAccountAliases, ["codex=work", "opencode=zen"])
    assert.deepEqual(report.checks.artifacts.missingArtifactProviderAccountAliases, [])
    assert.deepEqual(report.checks.artifacts.aggregate.providerAccountAliases, {
      "codex=work": 1,
      "opencode=zen": 1,
    })
    assert.deepEqual(report.checks.artifacts.requiredArtifactValidationPresets, ["cloud-distributed-runtime", "distributed-runtime"])
    assert.deepEqual(report.checks.artifacts.missingArtifactValidationPresets, [])
    assert.deepEqual(report.checks.artifacts.aggregate.validationPresets, {
      "cloud-distributed-runtime": 1,
      "distributed-runtime": 1,
    })
    assert.deepEqual(report.checks.artifacts.requiredArtifactPlannedOwners, ["validation-platform"])
    assert.deepEqual(report.checks.artifacts.missingArtifactPlannedOwners, [])
    assert.deepEqual(report.checks.artifacts.requiredArtifactPlannedClassifications, ["workspace-live-sync-conflict"])
    assert.deepEqual(report.checks.artifacts.missingArtifactPlannedClassifications, [])
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
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 2)
    assert.equal(artifactIndex.metadata.drill, "distributed-runtime-gate")
    assert.equal(artifactIndex.metadata.preset, "distributed-runtime")
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "codex=work,opencode=zen")
    assert.equal(artifactIndex.metadata.matrixEvidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.artifactEvidenceRepos, "cloud,oss")
    const indexedCoverageAreas = artifactIndex.metadata.coverageAreas.split(",")
    assert.equal(indexedCoverageAreas.includes("distributed-observability"), true)
    assert.equal(indexedCoverageAreas.includes("suite-contract"), true)
    assert.equal(indexedCoverageAreas.includes("matrix-validation"), true)
    const indexedRuntimeSignals = artifactIndex.metadata.runtimeSignals.split(",")
    assert.equal(indexedRuntimeSignals.includes("home-extension-manifest-sync"), true)
    assert.equal(indexedRuntimeSignals.includes("provider-run-lifecycle"), true)
    assert.equal(indexedRuntimeSignals.includes("runtime-projection-health"), true)
    assert.equal(indexedRuntimeSignals.includes("slice-auth-state"), true)
    assert.equal(indexedRuntimeSignals.includes("workspace-live-sync-state"), true)
    const indexedRuntimeSignalOwners = artifactIndex.metadata.runtimeSignalOwners.split(",")
    assert.equal(indexedRuntimeSignalOwners.includes("kernel-authority"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("provider-account"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("provider-runtime"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("runtime-state"), true)
    const indexedClassifications = artifactIndex.metadata.classifications.split(",")
    assert.equal(indexedClassifications.includes("kernel-authority"), true)
    assert.equal(indexedClassifications.includes("remote-extension-sync"), true)
    assert.equal(indexedClassifications.includes("workspace-live-sync-conflict"), true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires default artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

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
        assert.equal(report.checks.artifacts.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(report.checks.artifacts.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        return true
      },
    )

    const discovered = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(discovered.status, "passed")
    assert.equal(discovered.checks.artifacts.status, "passed")
    assert.equal(discovered.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 2)
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactSchemas, [])
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactEvidenceRepos, ["cloud", "oss"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactEvidenceRepos, [])
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactCoverageAreas, [])
    assert.deepEqual(discovered.checks.artifacts.aggregate.evidenceRepos, { cloud: 1, oss: 1 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate accepts explicit artifact evidence inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const explicitArtifactRoot = path.join(rootDir, "validation-artifacts", "cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    const ossArtifactIndex = await writeValidationSuiteArtifact(path.join(rootDir, "validation-artifacts", "oss"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(explicitArtifactRoot)

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      ossArtifactIndex,
      "--artifact-root",
      explicitArtifactRoot,
      "--json",
    ])).stdout)

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.roots, [explicitArtifactRoot])
    assert.deepEqual(report.checks.artifacts.inputs, [ossArtifactIndex])
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.cloud, 1)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.oss, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can run validation suites as artifact evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--json",
    ])).stdout)

    const expectedArtifactIndexes = [
      path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
      path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
    ]
    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, expectedArtifactIndexes)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.cloud, 1)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.oss, 1)
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 2)
    assert.deepEqual(report.generatedEvidence.validationSuites, {
      artifactIndexes: expectedArtifactIndexes.sort(),
      failureRoots: [
        path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
        path.join(validationSuiteOutputRoot, "oss", "failed-run"),
      ].sort(),
      commands: [
        {
          artifactIndexPath: path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
          args: [
            "--run-json",
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "oss", "failed-run"),
          ],
          cwd: ossRoot,
          failureRoot: path.join(validationSuiteOutputRoot, "oss", "failed-run"),
          nodeArgs: [
            path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
            "--run-json",
            "--output",
            path.join(validationSuiteOutputRoot, "oss", "drill-validation-suite-run.json"),
            "--output-artifact-index",
            path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "oss", "failed-run"),
          ],
          reportPath: path.join(validationSuiteOutputRoot, "oss", "drill-validation-suite-run.json"),
          scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
        },
        {
          artifactIndexPath: path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
          args: [
            "--run-json",
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
          ],
          cwd: cloudRoot,
          failureRoot: path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
          nodeArgs: [
            path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
            "--run-json",
            "--output",
            path.join(validationSuiteOutputRoot, "cloud", "cloud-validation-suite-run.json"),
            "--output-artifact-index",
            path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
            "--preserve-failure-root",
            path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
          ],
          reportPath: path.join(validationSuiteOutputRoot, "cloud", "cloud-validation-suite-run.json"),
          scriptPath: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
        },
      ],
      enabled: true,
      outputRoots: [
        path.join(validationSuiteOutputRoot, "cloud"),
        path.join(validationSuiteOutputRoot, "oss"),
      ].sort(),
    })
    assert.equal(report.generatedEvidence.matrixReports.enabled, false)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can run matrix reports as evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const matrixOutputRoot = path.join(rootDir, "generated-matrices")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    const outputPath = path.join(rootDir, "distributed-runtime-gate.json")
    const artifactIndexPath = path.join(rootDir, "distributed-runtime-gate-artifacts.json")
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })
    await writeFakeDistributedRuntimeMatrixScripts({ cloudRoot, ossRoot })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--run-matrix-reports",
      "--matrix-output-root",
      matrixOutputRoot,
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--require-artifact-provider-account-alias",
      "codex=work_codex",
      "--require-artifact-provider-account-alias",
      "opencode=zen",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
      "--json",
    ])).stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, report)
    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactProviderAccountAliases, ["codex=work_codex", "opencode=zen"])
    assert.deepEqual(report.checks.artifacts.missingArtifactProviderAccountAliases, [])
    assert.deepEqual(report.checks.artifacts.aggregate.providerAccountAliases, {
      "claude=work_claude": 4,
      "codex=work_codex": 5,
      "opencode=zen": 5,
    })
    assert.equal(report.checks.matrices.status, "passed")
    assert.deepEqual(report.checks.matrices.roots, [
      path.join(matrixOutputRoot, "cloud"),
      path.join(matrixOutputRoot, "oss"),
    ].sort())
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingProviders, [])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["workspace-live-sync-matrix"], 1)
    assert.equal(report.generatedEvidence.matrixReports.enabled, true)
    assert.deepEqual(report.generatedEvidence.matrixReports.limitations, [])
    assert.deepEqual(report.generatedEvidence.matrixReports.roots, [
      path.join(matrixOutputRoot, "cloud"),
      path.join(matrixOutputRoot, "oss"),
    ].sort())
    assert.deepEqual(report.generatedEvidence.matrixReports.artifactIndexes, [
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-agent-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-home-extension-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "workspace-live-sync-matrix-artifacts.json"),
    ])
    assert.equal(report.generatedEvidence.matrixReports.commands.length, 6)
    assert.equal(report.generatedEvidence.matrixReports.commands[0].reportPath, path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix.json"))
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[0].nodeArgs, [
      path.join(ossRoot, "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-hetzner",
      "--report",
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix.json"),
      "--artifact-index",
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix-artifacts.json"),
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[5].nodeArgs.slice(-4), [
      "--report",
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix.json"),
      "--output-artifact-index",
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json"),
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[0].args, [
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-hetzner",
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[1].args, [
      "--provider-account",
      "claude=work_claude",
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-hetzner",
      "--include-hosted-cloud",
    ])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[2].args, ["--include-hetzner"])
    assert.deepEqual(report.generatedEvidence.matrixReports.commands[4].args, [
      "--provider-account",
      "codex=work_codex",
      "--provider-account",
      "opencode=zen",
      "--include-remote",
      "--include-hetzner",
      "--include-opencode",
    ])
    assert.deepEqual(report.generatedEvidence.validationSuites.artifactIndexes, [
      path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
      path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
    ].sort())
    assert.equal(artifactIndex.metadata.generatedMatrixArtifactIndexes, [
      path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "native-provider-tui-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-agent-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "remote-home-extension-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "slice-runtime-matrix-artifacts.json"),
      path.join(matrixOutputRoot, "oss", "workspace-live-sync-matrix-artifacts.json"),
    ].join(","))
    assert.equal(artifactIndex.metadata.generatedEvidenceKinds, "matrix-report,validation-suite-run")
    assert.equal(artifactIndex.metadata.generatedMatrixNames, [
      "cloud-slice-runtime-matrix",
      "native-provider-tui-matrix",
      "remote-agent-runtime-matrix",
      "remote-home-extension-matrix",
      "slice-runtime-matrix",
      "workspace-live-sync-matrix",
    ].join(","))
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.generatedEvidenceRepos, "cloud,external,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "claude=work_claude,codex=work_codex,opencode=zen")
    assert.equal(artifactIndex.metadata.generatedMatrixRoots, [
      path.join(matrixOutputRoot, "cloud"),
      path.join(matrixOutputRoot, "oss"),
    ].sort().join(","))
    assert.equal(artifactIndex.metadata.generatedValidationSuiteRoots, [
      path.join(validationSuiteOutputRoot, "cloud"),
      path.join(validationSuiteOutputRoot, "oss"),
    ].sort().join(","))
    assert.equal(artifactIndex.metadata.generatedValidationSuiteFailureRoots, [
      path.join(validationSuiteOutputRoot, "cloud", "failed-run"),
      path.join(validationSuiteOutputRoot, "oss", "failed-run"),
    ].sort().join(","))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate labels dry-run generated matrix limitations", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const matrixOutputRoot = path.join(rootDir, "generated-matrices")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    const outputPath = path.join(rootDir, "distributed-runtime-gate.json")
    const artifactIndexPath = path.join(rootDir, "distributed-runtime-gate-artifacts.json")
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })
    await writeFakeDistributedRuntimeMatrixScripts({ cloudRoot, ossRoot })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--run-matrix-reports",
      "--matrix-dry-run",
      "--matrix-output-root",
      matrixOutputRoot,
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
      "--json",
    ])).stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    const summary = JSON.parse((await execFile(process.execPath, [
      summaryScriptPath,
      "--gate-report",
      outputPath,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--require-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
    ])).stdout)

    assert.equal(report.status, "passed")
    assert.equal(report.generatedEvidence.matrixReports.dryRun, true)
    assert.deepEqual(report.generatedEvidence.matrixReports.limitations, [{
      kind: "dry-run-classification-coverage",
      owner: "validation-harness",
      nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before treating required matrix classifications as release evidence",
    }])
    assert.equal(artifactIndex.metadata.generatedMatrixLimitations, "dry-run-classification-coverage")
    assert.match(artifactIndex.metadata.generatedMatrixNames, /cloud-slice-runtime-matrix/)
    assert.match(artifactIndex.metadata.generatedMatrixNames, /workspace-live-sync-matrix/)
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "cloud,oss")
    assert.match(artifactIndex.metadata.generatedMatrixArtifactIndexes, /cloud-slice-runtime-matrix-artifacts\.json/)
    assert.match(artifactIndex.metadata.generatedMatrixArtifactIndexes, /workspace-live-sync-matrix-artifacts\.json/)
    assert.equal(summary.status, "passed")
    assert.equal(summary.coverage.generatedMatrixArtifactIndexes[path.join(matrixOutputRoot, "cloud", "cloud-slice-runtime-matrix-artifacts.json")], 1)
    assert.equal(summary.coverage.generatedMatrixArtifactIndexes[path.join(matrixOutputRoot, "oss", "workspace-live-sync-matrix-artifacts.json")], 1)
    assert.deepEqual(summary.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(summary.missingArtifactGeneratedMatrixLimitations, [])
    assert.deepEqual(summary.requiredGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(summary.missingGeneratedMatrixLimitations, [])
    assert(report.generatedEvidence.matrixReports.commands.every((command) => command.args.includes("--dry-run")))
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires executed Cloud validation suite artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteManifestArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(report.checks.artifacts.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite.v1"], 1)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires Cloud validation suite distributed observability coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.artifacts.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
        assert.deepEqual(report.checks.artifacts.missingArtifactCoverageAreas, ["distributed-observability"])
        assert.match(report.checks.artifacts.error, /missing required artifact coverage areas: distributed-observability/)
        assert.equal(report.checks.artifacts.aggregate.coverageAreas["suite-contract"], 2)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can include default failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeFailureManifest(path.join(cloudRoot, ".artifacts", "failed-run", "arroba-drill-failure.json"), {
      drill: "cloud-slice-runtime-matrix",
      message: "slice auth stale projection",
    })

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(skipped.status, "passed")
    assert.equal(skipped.checks.failures.status, "skipped")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--include-default-failures",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.status, "failed")
        assert.deepEqual(report.checks.failures.roots, [
          path.join(cloudRoot, ".artifacts"),
          path.join(ossRoot, ".artifacts"),
        ].sort())
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "cloud-slice-runtime-matrix")
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate accepts explicit failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    const failureManifest = await writeFailureManifest(path.join(rootDir, "preserved", "arroba-drill-failure.json"), {
      drill: "remote-agent-runtime-matrix",
      message: "worker lease expired",
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--failure-manifest",
        failureManifest,
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.status, "failed")
        assert.deepEqual(report.checks.failures.inputs, [failureManifest])
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "remote-agent-runtime-matrix")
        return true
      },
    )
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
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        assert.deepEqual(report.checks.matrices.missingScenarios, ["hosted-collab-remote-agent", "hosted-single-user-remote-agent", "ui-projection"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
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

test("distributed runtime gate can require generated matrix registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudGeneratedMatrixRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-generated-matrix-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects generated matrix registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [
        { name: "cloud-slice-runtime-matrix", repo: "cloud" },
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-generated-matrix-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /generated matrix registry parity failed/)
        assert.match(error.stderr, /workspace-live-sync-matrix/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can require runtime signal registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudRuntimeSignalsRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-runtime-signal-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can require failure taxonomy registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudFailureTaxonomyRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-failure-taxonomy-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects failure taxonomy registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications
        .filter((classification) => classification.kind === "kernel-authority")
        .map((classification) => ({ ...classification, owner: "runtime-state" })),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-failure-taxonomy-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /failure taxonomy registry parity failed/)
        assert.match(error.stderr, /kernel-authority/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects runtime signal registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeSignalsManifest()
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: manifest.signals.filter((signal) => signal.id !== "home-extension-manifest-sync"),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-runtime-signal-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /runtime signal registry parity failed/)
        assert.match(error.stderr, /home-extension-manifest-sync/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects unsupported provider account aliases", async () => {
  for (const args of [
    ["--provider-account", "dev-stub=stub"],
    ["--provider-account=claude-headless=headless"],
  ]) {
    const rawAlias = args.at(-1).replace("--provider-account=", "")
    await assert.rejects(
      execFile(process.execPath, [scriptPath, ...args, "--json"]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /unsupported distributed-runtime provider account alias provider/)
        assert.doesNotMatch(error.stderr, new RegExp(rawAlias))
        assert.doesNotMatch(error.stdout, new RegExp(rawAlias))
        return true
      },
    )
  }
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
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-failure-max-age-ms", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-failure-max-age-ms requires a value/)
      return true
    },
  )
})

test("distributed runtime gate rejects aggregate-only generated evidence requirements", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-evidence-kind", "matrix-report", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-evidence-kind is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-matrix-limitation", "dry-run-classification-coverage", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-matrix-limitation is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-validation-suite-failure-root", "/tmp/generated-suite/failed-run", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-validation-suite-failure-root is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
})

test("distributed runtime gate rejects secret-looking generated output roots", async () => {
  for (const flag of ["--validation-suite-output-root", "--matrix-output-root"]) {
    await assert.rejects(
      execFile(process.execPath, [scriptPath, flag, "/tmp/Bearer abcdefghijklmnop", "--json"]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, new RegExp(`${flag} includes secret-looking generated evidence path`))
        assert.doesNotMatch(error.stderr, /Bearer abcdefghijklmnop/)
        assert.doesNotMatch(error.stdout, /Bearer abcdefghijklmnop/)
        return true
      },
    )
  }
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
      scenario("transcript-parity", "provider-error", ["client-projection-health", "runtime-projection-health"]),
      scenario("provider-auth-health", "provider-auth", ["provider-run-lifecycle"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-agent-runtime.json"), {
    matrix: "remote-agent-runtime-matrix",
    metadata: {
      deploymentPresets: includeCloud
        ? "hetzner,hosted-cloud,same-host-remote,self-hosted-relay"
        : "hetzner,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
      scenario("hetzner-collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
      scenario("hetzner-single-user-remote-agent", "worker-execution", ["agent-lifecycle", "lease-health", "session-authority"]),
      ...(includeCloud
        ? [
          scenario("hosted-collab-remote-agent", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
          scenario("hosted-single-user-remote-agent", "relay-runtime", ["agent-lifecycle", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "relay-target-freshness"]),
        ]
        : []),
      scenario("lease-reconnect", "relay-target-freshness", ["lease-health", "relay-target-freshness"]),
      scenario("provider-run-binding", "worker-execution", ["lease-health", "provider-run-lifecycle"]),
      scenario("remote-prompt-dispatch", "relay-runtime", ["agent-lifecycle", "provider-run-lifecycle"]),
      scenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "runtime-projection-health", "session-authority"]),
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
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
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

async function writeValidationSuiteArtifact(rootDir, {
  coverageAreas = ["distributed-observability", "suite-contract"],
  evidenceRepo = "cloud",
  providerAccountAliases = "",
  plannedOwners = "",
  plannedClassifications = "",
  validationPresets = evidenceRepo === "oss" ? "distributed-runtime" : "cloud-distributed-runtime",
} = {}) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    exitCode: 0,
    signal: null,
    error: null,
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    manifest: {
      schema: "arroba.drill.validation_suite.v1",
      testCount: 1,
      command: "node --test scripts/cloud-validation-suite.test.mjs",
      coverage: [{
        id: "suite-contract",
        description: "Cloud validation-suite contract",
        testCount: 1,
        testPaths: ["scripts/cloud-validation-suite.test.mjs"],
      }],
      failureTaxonomyManifest: drillFailureTaxonomyManifest(),
      runtimeSignalsManifest: drillRuntimeSignalsManifest(),
      testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    },
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      coverageAreas: coverageAreas.join(","),
      runtimeSignals: DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS.join(","),
      runtimeSignalOwners: drillRuntimeSignalOwnersFor(DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS).join(","),
      owners: "validation-platform",
      classifications: evidenceRepo === "cloud" ? "cloud-validation-suite" : "validation-suite",
      requiredFailureClassifications: DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS.join(","),
      artifactKinds: "validation-suite-run",
      evidenceRepos: evidenceRepo,
      generatedMatrixNames: generatedMatrixNamesForEvidenceRepo(evidenceRepo).join(","),
      generatedMatrixRepos: evidenceRepo,
      validationPresets,
      ...(providerAccountAliases ? { providerAccountAliases } : {}),
      ...(plannedOwners ? { plannedOwners } : {}),
      ...(plannedClassifications ? { plannedClassifications } : {}),
      exitCriterionStatuses: "satisfied",
    },
  })
  return path.join(rootDir, "arroba-drill-artifacts.json")
}

function generatedMatrixNamesForEvidenceRepo(evidenceRepo) {
  return DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO[evidenceRepo] ?? []
}

async function writeCloudGeneratedMatrixRegistry(cloudRoot, {
  matrices = [
    { name: "cloud-slice-runtime-matrix", repo: "cloud" },
    { name: "native-provider-tui-matrix", repo: "oss" },
    { name: "remote-agent-runtime-matrix", repo: "oss" },
    { name: "remote-home-extension-matrix", repo: "oss" },
    { name: "slice-runtime-matrix", repo: "oss" },
    { name: "workspace-live-sync-matrix", repo: "oss" },
  ],
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudDrillGeneratedMatrixNamesManifest() {",
    `  return { schema: "arroba.cloud.drill.generated_matrix_names.v1", matrices: ${JSON.stringify(matrices)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

async function writeCloudRuntimeSignalsRegistry(cloudRoot, {
  signals = drillRuntimeSignalsManifest().signals,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeSignalsManifest() {",
    `  return { schema: "arroba.drill.runtime_signals.v1", signals: ${JSON.stringify(signals)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

async function writeCloudFailureTaxonomyRegistry(cloudRoot, {
  classifications = drillFailureTaxonomyManifest().classifications
    .filter((classification) => [
      "docker-runtime",
      "kernel-authority",
      "runtime-projection-health",
      "workspace-live-sync-conflict",
    ].includes(classification.kind))
    .map((classification) => classification.kind === "docker-runtime"
      ? { ...classification, owner: "worker-kernel" }
      : classification),
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudFailureTaxonomyManifest() {",
    `  return { schema: "arroba.drill.failure_taxonomy.v1", target: "scenario", classifications: ${JSON.stringify(classifications)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

const DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS = Object.freeze([
  "agent-lifecycle",
  "client-projection-health",
  "home-extension-manifest-sync",
  "lease-health",
  "permission-interaction",
  "provider-run-lifecycle",
  "relay-target-freshness",
  "runtime-projection-health",
  "session-authority",
  "slice-auth-state",
  "slice-runtime-state",
  "workspace-live-sync-state",
])

const DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS = Object.freeze([
  "cloud-runtime",
  "docker-runtime",
  "kernel-authority",
  "provider-auth",
  "provider-error",
  "projection-staleness",
  "relay-runtime",
  "relay-target-freshness",
  "remote-extension-sync",
  "remote-host-capacity",
  "remote-worker-version",
  "runtime-projection-health",
  "slice-auth",
  "slice-runtime",
  "ui-client-projection",
  "worker-execution",
  "workspace-live-sync-conflict",
])

async function writeValidationSuiteManifestArtifact(rootDir) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite.v1",
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    coverage: [{
      id: "suite-contract",
      description: "Cloud validation-suite contract",
      testCount: 1,
      testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    }],
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      artifactKinds: "validation-suite",
      evidenceRepos: "cloud",
    },
  })
}

async function writeFailureManifest(file, {
  drill = "failed-drill",
  message = "Token refresh failed: 401",
} = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill },
    error: { name: "Error", message, stack: null },
  }, null, 2)}\n`, "utf8")
  return file
}

async function writeFakeDistributedRuntimeMatrixScripts({ cloudRoot, ossRoot }) {
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
    report: matrixReport({
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
        scenario("transcript-parity", "provider-error", ["client-projection-health", "runtime-projection-health"]),
        scenario("provider-auth-health", "provider-auth", ["provider-run-lifecycle"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-remote-agent-runtime-matrix-drill.mjs"),
    report: matrixReport({
      matrix: "remote-agent-runtime-matrix",
      metadata: {
        deploymentPresets: "hetzner,hosted-cloud,same-host-remote,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
        scenario("hetzner-collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
        scenario("hetzner-single-user-remote-agent", "worker-execution", ["agent-lifecycle", "lease-health", "session-authority"]),
        scenario("hosted-collab-remote-agent", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
        scenario("hosted-single-user-remote-agent", "relay-runtime", ["agent-lifecycle", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "relay-target-freshness"]),
        scenario("lease-reconnect", "relay-target-freshness", ["lease-health", "relay-target-freshness"]),
        scenario("provider-run-binding", "worker-execution", ["lease-health", "provider-run-lifecycle"]),
        scenario("remote-prompt-dispatch", "relay-runtime", ["agent-lifecycle", "provider-run-lifecycle"]),
        scenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "runtime-projection-health", "session-authority"]),
      ],
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-remote-home-extension-matrix-drill.mjs"),
    report: matrixReport({
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
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-slice-runtime-matrix-drill.mjs"),
    report: matrixReport({
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
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(ossRoot, "apps", "cli", "scripts", "live-workspace-live-sync-matrix-drill.mjs"),
    report: matrixReport({
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
    }),
  })
  await writeFakeMatrixScript({
    file: path.join(cloudRoot, "scripts", "staging-slice-runtime-matrix.mjs"),
    report: matrixReport({
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    }),
  })
}

function matrixReport({ matrix, metadata, scenarios }) {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios,
  }
}

async function writeFakeMatrixScript({ file, report }) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `#!/usr/bin/env node
import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

const args = process.argv.slice(2)
const reportPath = valueFor("--report")
const artifactIndexPath = valueFor("--artifact-index") ?? valueFor("--output-artifact-index")
await mkdir(path.dirname(reportPath), { recursive: true })
const baseReport = ${JSON.stringify(report, null, 2)}
const providerAccountAliases = providerAccountAliasesFor(args)
const reportWithMetadata = providerAccountAliases.length > 0
  ? {
    ...baseReport,
    metadata: {
      ...baseReport.metadata,
      providerAccountAliases: providerAccountAliases.join(","),
    },
  }
  : baseReport
const report = args.includes("--dry-run") ? dryRunReportFor(reportWithMetadata) : reportWithMetadata
await writeFile(reportPath, \`\${JSON.stringify(report, null, 2)}\\n\`, "utf8")
if (artifactIndexPath) {
  const bytes = await readFile(reportPath)
  const index = {
    schema: "arroba.drill.artifact_index.v1",
    rootDir: path.dirname(reportPath),
    createdAt: "2026-06-13T00:00:02.000Z",
    metadata: {
      drill: report.matrix,
      matrix: report.matrix,
      artifactKinds: "matrix-report",
      generatedMatrixNames: report.matrix,
      generatedMatrixRepos: report.matrix.startsWith("cloud-") ? "cloud" : "oss",
      ...(providerAccountAliases.length > 0 ? { providerAccountAliases: providerAccountAliases.join(",") } : {}),
    },
    artifacts: [{
      path: path.basename(reportPath),
      schema: report.schema,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      sizeBytes: bytes.byteLength,
    }],
  }
  await writeFile(artifactIndexPath, \`\${JSON.stringify(index, null, 2)}\\n\`, "utf8")
}

function valueFor(flag) {
  const index = args.indexOf(flag)
  if (index < 0 || !args[index + 1]) return null
  return args[index + 1]
}

function providerAccountAliasesFor(args) {
  const aliases = []
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === "--provider-account" && args[index + 1]) {
      aliases.push(args[index + 1])
      index += 1
    } else if (args[index].startsWith("--provider-account=")) {
      aliases.push(args[index].slice("--provider-account=".length))
    }
  }
  return aliases.sort()
}

function dryRunReportFor(report) {
  return {
    ...report,
    status: "dry-run",
    dryRun: true,
    completedAt: report.startedAt,
    durationMs: 0,
    scenarios: report.scenarios.map((scenario) => ({
      ...scenario,
      status: "dry-run",
      ok: true,
      classification: null,
      durationMs: 0,
      reason: null,
    })),
  }
}
`, "utf8")
}

async function writeFakeValidationSuiteScript({
  classification,
  evidenceRepo,
  file,
}) {
  const validationPresets = evidenceRepo === "oss" ? "distributed-runtime" : "cloud-distributed-runtime"
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `#!/usr/bin/env node
import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

const args = process.argv.slice(2)
const outputPath = valueFor("--output")
const artifactIndexPath = valueFor("--output-artifact-index")
const outputDir = path.dirname(outputPath)
await mkdir(outputDir, { recursive: true })
const report = {
  schema: "arroba.drill.validation_suite_run.v1",
  status: "passed",
  ok: true,
  startedAt: "2026-06-13T00:00:00.000Z",
  completedAt: "2026-06-13T00:00:01.000Z",
  durationMs: 1000,
  exitCode: 0,
  signal: null,
  error: null,
  command: "node --test fake-validation-suite.test.mjs",
  testCount: 1,
  testPaths: ["fake-validation-suite.test.mjs"],
  manifest: {
    schema: "arroba.drill.validation_suite.v1",
    testCount: 1,
    command: "node --test fake-validation-suite.test.mjs",
    coverage: [
      {
        id: "distributed-observability",
        description: "Distributed observability evidence.",
        testCount: 1,
        testPaths: ["fake-validation-suite.test.mjs"],
      },
      {
        id: "suite-contract",
        description: "Suite contract evidence.",
        testCount: 1,
        testPaths: ["fake-validation-suite.test.mjs"],
      },
    ],
    failureTaxonomyManifest: ${JSON.stringify(drillFailureTaxonomyManifest())},
    runtimeSignalsManifest: ${JSON.stringify(drillRuntimeSignalsManifest())},
    testPaths: ["fake-validation-suite.test.mjs"],
  },
}
await writeFile(outputPath, \`\${JSON.stringify(report, null, 2)}\\n\`, "utf8")
const bytes = await readFile(outputPath)
const index = {
  schema: "arroba.drill.artifact_index.v1",
  rootDir: outputDir,
  createdAt: "2026-06-13T00:00:02.000Z",
  metadata: {
    drill: "validation-suite",
    tests: 1,
    coverageAreas: "distributed-observability,suite-contract",
    runtimeSignals: ${JSON.stringify(DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS.join(","))},
    runtimeSignalOwners: "kernel-authority,provider-account,provider-runtime,runtime-network,runtime-state,ui-client,worker-kernel",
    owners: "validation-platform",
    classifications: ${JSON.stringify(classification)},
    requiredFailureClassifications: ${JSON.stringify(DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS.join(","))},
    artifactKinds: "validation-suite-run",
    evidenceRepos: ${JSON.stringify(evidenceRepo)},
    generatedMatrixNames: ${JSON.stringify(generatedMatrixNamesForEvidenceRepo(evidenceRepo).join(","))},
    generatedMatrixRepos: ${JSON.stringify(evidenceRepo)},
    validationPresets: ${JSON.stringify(validationPresets)},
    exitCriterionStatuses: "satisfied",
  },
  artifacts: [{
    path: path.basename(outputPath),
    schema: report.schema,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    sizeBytes: bytes.byteLength,
  }],
}
await writeFile(artifactIndexPath, \`\${JSON.stringify(index, null, 2)}\\n\`, "utf8")

function valueFor(flag) {
  const index = args.indexOf(flag)
  if (index < 0 || !args[index + 1]) throw new Error(\`\${flag} requires a value\`)
  return args[index + 1]
}
`, "utf8")
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
