import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  distributedRuntimeGeneratedEvidenceSummaryFor,
  distributedRuntimeMatrixCommandsFor,
  distributedRuntimeMatrixOutputDirFor,
  distributedRuntimeValidationSuiteOutputDirFor,
} from "./drill-distributed-runtime-evidence.mjs"

test("builds distributed runtime matrix command contracts", () => {
  const commands = distributedRuntimeMatrixCommandsFor({
    cloudOutputDir: "/tmp/out/cloud",
    cloudRoot: "/repo/arroba-cloud",
    commonArgs: ["--dry-run", "--continue-on-failure"],
    ossOutputDir: "/tmp/out/oss",
    ossRoot: "/repo/arroba",
  })

  assert.deepEqual(commands.map((command) => ({
    artifactIndexFlag: command.artifactIndexFlag,
    cwd: command.cwd,
    reportFileName: command.reportFileName,
    scriptPath: command.scriptPath,
  })), [
    {
      artifactIndexFlag: "--artifact-index",
      cwd: "/repo/arroba",
      reportFileName: "native-provider-tui-matrix.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      cwd: "/repo/arroba",
      reportFileName: "remote-agent-runtime-matrix.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-remote-agent-runtime-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      cwd: "/repo/arroba",
      reportFileName: "remote-home-extension-matrix.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-remote-home-extension-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      cwd: "/repo/arroba",
      reportFileName: "slice-runtime-matrix.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-slice-runtime-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      cwd: "/repo/arroba",
      reportFileName: "workspace-live-sync-matrix.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-workspace-live-sync-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--output-artifact-index",
      cwd: "/repo/arroba-cloud",
      reportFileName: "cloud-slice-runtime-matrix.json",
      scriptPath: path.join("/repo/arroba-cloud", "scripts", "staging-slice-runtime-matrix.mjs"),
    },
  ])
  assert.deepEqual(commands[0].args, ["--dry-run", "--continue-on-failure", "--include-hetzner"])
  assert.deepEqual(commands[1].args, ["--dry-run", "--continue-on-failure", "--include-hetzner", "--include-hosted-cloud"])
  assert.deepEqual(commands[5].args, ["--dry-run", "--continue-on-failure", "--include-hosted-cloud", "--include-vault"])
})

test("builds distributed runtime generated evidence output directories", () => {
  const options = {
    cloudRoot: "/repo/arroba-cloud",
    matrixOutputRoot: null,
    ossRoot: "/repo/arroba",
    validationSuiteOutputRoot: null,
  }

  assert.equal(
    distributedRuntimeMatrixOutputDirFor(options, "oss"),
    path.join("/repo/arroba", ".artifacts", "drill-matrices", "distributed-runtime-gate"),
  )
  assert.equal(
    distributedRuntimeMatrixOutputDirFor(options, "cloud"),
    path.join("/repo/arroba-cloud", ".artifacts", "drill-matrices", "distributed-runtime-gate"),
  )
  assert.equal(
    distributedRuntimeValidationSuiteOutputDirFor(options, "oss"),
    path.join("/repo/arroba", ".artifacts", "validation-suite", "distributed-runtime-gate"),
  )
  assert.equal(
    distributedRuntimeValidationSuiteOutputDirFor(options, "cloud"),
    path.join("/repo/arroba-cloud", ".artifacts", "validation-suite", "distributed-runtime-gate"),
  )
  assert.equal(
    distributedRuntimeMatrixOutputDirFor({ ...options, matrixOutputRoot: "/tmp/matrices" }, "cloud"),
    path.join("/tmp/matrices", "cloud"),
  )
  assert.equal(
    distributedRuntimeValidationSuiteOutputDirFor({ ...options, validationSuiteOutputRoot: "/tmp/suites" }, "oss"),
    path.join("/tmp/suites", "oss"),
  )
})

test("builds distributed runtime generated evidence summary", () => {
  const options = {
    cloudRoot: "/repo/arroba-cloud",
    matrixContinueOnFailure: true,
    matrixDryRun: true,
    matrixOutputRoot: "/tmp/matrices",
    ossRoot: "/repo/arroba",
    runMatrixReports: true,
    runValidationSuites: true,
    validationSuiteOutputRoot: "/tmp/suites",
  }

  const summary = distributedRuntimeGeneratedEvidenceSummaryFor(options, {
    generatedMatrixRoots: ["/tmp/matrices/oss", "/tmp/matrices/cloud"],
    validationSuiteArtifactIndexes: [
      "/tmp/suites/oss/arroba-drill-artifacts.json",
      "/tmp/suites/cloud/arroba-drill-artifacts.json",
    ],
  })

  assert.equal(summary.matrixReports.enabled, true)
  assert.equal(summary.matrixReports.dryRun, true)
  assert.equal(summary.matrixReports.continueOnFailure, true)
  assert.deepEqual(summary.matrixReports.roots, ["/tmp/matrices/cloud", "/tmp/matrices/oss"])
  assert.equal(summary.matrixReports.commands.length, 6)
  assert.deepEqual(summary.matrixReports.commands[0], {
    artifactIndexPath: path.join("/tmp/matrices/oss", "native-provider-tui-matrix-artifacts.json"),
    args: ["--dry-run", "--continue-on-failure", "--include-hetzner"],
    cwd: "/repo/arroba",
    reportPath: path.join("/tmp/matrices/oss", "native-provider-tui-matrix.json"),
    scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
  })
  assert.deepEqual(summary.validationSuites, {
    artifactIndexes: [
      "/tmp/suites/cloud/arroba-drill-artifacts.json",
      "/tmp/suites/oss/arroba-drill-artifacts.json",
    ],
    enabled: true,
    outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
  })
})

test("builds empty generated evidence summary when generation is disabled", () => {
  const summary = distributedRuntimeGeneratedEvidenceSummaryFor({
    cloudRoot: "/repo/arroba-cloud",
    matrixContinueOnFailure: false,
    matrixDryRun: false,
    ossRoot: "/repo/arroba",
    runMatrixReports: false,
    runValidationSuites: false,
  })

  assert.deepEqual(summary, {
    matrixReports: {
      commands: [],
      continueOnFailure: false,
      dryRun: false,
      enabled: false,
      roots: [],
    },
    validationSuites: {
      artifactIndexes: [],
      enabled: false,
      outputRoots: [],
    },
  })
})
