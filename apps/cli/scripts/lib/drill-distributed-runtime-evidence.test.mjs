import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_REPOS,
  distributedRuntimeGeneratedEvidenceSummaryFor,
  distributedRuntimeMatrixArtifactIndexPathsFor,
  distributedRuntimeMatrixCommandsFor,
  distributedRuntimeMatrixOutputDirFor,
  distributedRuntimeValidationSuiteCommandsFor,
  distributedRuntimeValidationSuiteOutputDirFor,
  runDistributedRuntimeMatrixReportCommand,
  runDistributedRuntimeValidationSuiteCommand,
} from "./drill-distributed-runtime-evidence.mjs"
import {
  DRILL_GENERATED_MATRIX_NAMES_BY_REPO,
} from "./drill-generated-matrix-names.mjs"

test("builds distributed runtime matrix command contracts", () => {
  const commands = distributedRuntimeMatrixCommandsFor({
    cloudOutputDir: "/tmp/out/cloud",
    cloudRoot: "/repo/arroba-cloud",
    commonArgs: ["--dry-run", "--continue-on-failure"],
    ossOutputDir: "/tmp/out/oss",
    ossRoot: "/repo/arroba",
    providerAccounts: {
      claude: "work_claude",
      codex: "work_codex",
      "dev-stub": "stub",
      opencode: "zen",
    },
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
      reportFileName: "runtime-resilience-chaos-matrix.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-runtime-resilience-chaos-matrix-drill.mjs"),
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
      reportFileName: "browser-terminal-resilience-matrix.json",
      scriptPath: path.join("/repo/arroba-cloud", "scripts", "browser-terminal-resilience-matrix.mjs"),
    },
    {
      artifactIndexFlag: "--output-artifact-index",
      cwd: "/repo/arroba-cloud",
      reportFileName: "cloud-slice-runtime-matrix.json",
      scriptPath: path.join("/repo/arroba-cloud", "scripts", "staging-slice-runtime-matrix.mjs"),
    },
  ])
  assert.deepEqual(commands[0].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "claude=work_claude",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-hetzner",
  ])
  assert.deepEqual(commands[1].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "claude=work_claude",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-hetzner",
    "--include-hosted-cloud",
  ])
  assert.deepEqual(commands[2].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "claude=work_claude",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-slices",
    "--include-hetzner",
    "--include-hosted-cloud",
  ])
  assert.deepEqual(commands[3].args, ["--dry-run", "--continue-on-failure", "--include-hetzner"])
  assert.deepEqual(commands[4].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "claude=work_claude",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-self-hosted-relay",
  ])
  assert.deepEqual(commands[5].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-remote",
    "--include-hetzner",
    "--include-opencode",
  ])
  assert.deepEqual(commands[6].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "claude=work_claude",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-hosted-cloud",
    "--include-hetzner",
  ])
  assert.deepEqual(commands[7].args, [
    "--dry-run",
    "--continue-on-failure",
    "--provider-account",
    "claude=work_claude",
    "--provider-account",
    "codex=work_codex",
    "--provider-account",
    "opencode=zen",
    "--include-hosted-cloud",
    "--include-vault",
  ])
  assert.deepEqual(commands.map((command) => `${command.repo}/${command.matrix}`), [
    "oss/native-provider-tui-matrix",
    "oss/remote-agent-runtime-matrix",
    "oss/runtime-resilience-chaos-matrix",
    "oss/remote-home-extension-matrix",
    "oss/slice-runtime-matrix",
    "oss/workspace-live-sync-matrix",
    "cloud/browser-terminal-resilience-matrix",
    "cloud/cloud-slice-runtime-matrix",
  ])
  assert.deepEqual(DISTRIBUTED_RUNTIME_GENERATED_MATRIX_REPOS, Object.keys(DRILL_GENERATED_MATRIX_NAMES_BY_REPO).sort())
  assert.deepEqual([...new Set(commands.map((command) => command.repo))].sort(), DISTRIBUTED_RUNTIME_GENERATED_MATRIX_REPOS)
  assert.deepEqual(commands.map((command) => command.matrix).sort(), [...DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES].sort())
  assert.deepEqual(
    commands.filter((command) => command.repo === "oss").map((command) => command.matrix),
    DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO.oss,
  )
  assert.deepEqual(
    commands.filter((command) => command.repo === "cloud").map((command) => command.matrix),
    DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO.cloud,
  )
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

test("builds distributed runtime validation suite command contracts", () => {
  const commands = distributedRuntimeValidationSuiteCommandsFor({
    cloudOutputDir: "/tmp/suites/cloud",
    cloudRoot: "/repo/arroba-cloud",
    ossOutputDir: "/tmp/suites/oss",
    ossRoot: "/repo/arroba",
  })

  assert.deepEqual(commands, [
    {
      cwd: "/repo/arroba",
      outputDir: "/tmp/suites/oss",
      preserveFailureRoot: path.join("/tmp/suites/oss", "failed-run"),
      reportFileName: "drill-validation-suite-run.json",
      scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    },
    {
      cwd: "/repo/arroba-cloud",
      outputDir: "/tmp/suites/cloud",
      preserveFailureRoot: path.join("/tmp/suites/cloud", "failed-run"),
      reportFileName: "cloud-validation-suite-run.json",
      scriptPath: path.join("/repo/arroba-cloud", "scripts", "cloud-validation-suite.mjs"),
    },
  ])
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
    providerAccounts: {
      codex: "work_codex",
    },
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
  assert.deepEqual(summary.matrixReports.limitations, [{
    kind: "dry-run-classification-coverage",
    owner: "validation-harness",
    nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before treating required matrix classifications as release evidence",
  }])
  assert.deepEqual(summary.matrixReports.roots, ["/tmp/matrices/cloud", "/tmp/matrices/oss"])
  assert.deepEqual(summary.matrixReports.artifactIndexes, [
    "/tmp/matrices/cloud/browser-terminal-resilience-matrix-artifacts.json",
    "/tmp/matrices/cloud/cloud-slice-runtime-matrix-artifacts.json",
    "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
    "/tmp/matrices/oss/remote-agent-runtime-matrix-artifacts.json",
    "/tmp/matrices/oss/remote-home-extension-matrix-artifacts.json",
    "/tmp/matrices/oss/runtime-resilience-chaos-matrix-artifacts.json",
    "/tmp/matrices/oss/slice-runtime-matrix-artifacts.json",
    "/tmp/matrices/oss/workspace-live-sync-matrix-artifacts.json",
  ])
  assert.equal(summary.matrixReports.commands.length, 8)
  assert.deepEqual(summary.matrixReports.commands[0], {
    artifactIndexFlag: "--artifact-index",
    artifactIndexPath: path.join("/tmp/matrices/oss", "native-provider-tui-matrix-artifacts.json"),
    args: ["--dry-run", "--continue-on-failure", "--provider-account", "codex=work_codex", "--include-hetzner"],
    cwd: "/repo/arroba",
    matrix: "native-provider-tui-matrix",
    nodeArgs: [
      path.join("/repo/arroba", "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
      "--dry-run",
      "--continue-on-failure",
      "--provider-account",
      "codex=work_codex",
      "--include-hetzner",
      "--report",
      path.join("/tmp/matrices/oss", "native-provider-tui-matrix.json"),
      "--artifact-index",
      path.join("/tmp/matrices/oss", "native-provider-tui-matrix-artifacts.json"),
    ],
    repo: "oss",
    reportPath: path.join("/tmp/matrices/oss", "native-provider-tui-matrix.json"),
    scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
  })
  assert.deepEqual(summary.matrixReports.commands.map((command) => command.artifactIndexFlag), [
    "--artifact-index",
    "--artifact-index",
    "--artifact-index",
    "--artifact-index",
    "--artifact-index",
    "--artifact-index",
    "--output-artifact-index",
    "--output-artifact-index",
  ])
  assert.deepEqual(summary.validationSuites, {
    artifactIndexes: [
      "/tmp/suites/cloud/arroba-drill-artifacts.json",
      "/tmp/suites/oss/arroba-drill-artifacts.json",
    ],
    failureRoots: [
      "/tmp/suites/cloud/failed-run",
      "/tmp/suites/oss/failed-run",
    ],
    commands: [
      {
        artifactIndexPath: path.join("/tmp/suites/oss", "arroba-drill-artifacts.json"),
        args: ["--run-json", "--preserve-failure-root", path.join("/tmp/suites/oss", "failed-run")],
        cwd: "/repo/arroba",
        failureRoot: path.join("/tmp/suites/oss", "failed-run"),
        nodeArgs: [
          path.join("/repo/arroba", "apps", "cli", "scripts", "drill-validation-suite.mjs"),
          "--run-json",
          "--output",
          path.join("/tmp/suites/oss", "drill-validation-suite-run.json"),
          "--output-artifact-index",
          path.join("/tmp/suites/oss", "arroba-drill-artifacts.json"),
          "--preserve-failure-root",
          path.join("/tmp/suites/oss", "failed-run"),
        ],
        reportPath: path.join("/tmp/suites/oss", "drill-validation-suite-run.json"),
        scriptPath: path.join("/repo/arroba", "apps", "cli", "scripts", "drill-validation-suite.mjs"),
      },
      {
        artifactIndexPath: path.join("/tmp/suites/cloud", "arroba-drill-artifacts.json"),
        args: ["--run-json", "--preserve-failure-root", path.join("/tmp/suites/cloud", "failed-run")],
        cwd: "/repo/arroba-cloud",
        failureRoot: path.join("/tmp/suites/cloud", "failed-run"),
        nodeArgs: [
          path.join("/repo/arroba-cloud", "scripts", "cloud-validation-suite.mjs"),
          "--run-json",
          "--output",
          path.join("/tmp/suites/cloud", "cloud-validation-suite-run.json"),
          "--output-artifact-index",
          path.join("/tmp/suites/cloud", "arroba-drill-artifacts.json"),
          "--preserve-failure-root",
          path.join("/tmp/suites/cloud", "failed-run"),
        ],
        reportPath: path.join("/tmp/suites/cloud", "cloud-validation-suite-run.json"),
        scriptPath: path.join("/repo/arroba-cloud", "scripts", "cloud-validation-suite.mjs"),
      },
    ],
    enabled: true,
    outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
  })
})

test("builds generated matrix artifact index inputs", () => {
  const paths = distributedRuntimeMatrixArtifactIndexPathsFor({
    cloudRoot: "/repo/arroba-cloud",
    matrixOutputRoot: "/tmp/matrices",
    ossRoot: "/repo/arroba",
    runMatrixReports: true,
  })

  assert.deepEqual(paths, [
    "/tmp/matrices/cloud/browser-terminal-resilience-matrix-artifacts.json",
    "/tmp/matrices/cloud/cloud-slice-runtime-matrix-artifacts.json",
    "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
    "/tmp/matrices/oss/remote-agent-runtime-matrix-artifacts.json",
    "/tmp/matrices/oss/remote-home-extension-matrix-artifacts.json",
    "/tmp/matrices/oss/runtime-resilience-chaos-matrix-artifacts.json",
    "/tmp/matrices/oss/slice-runtime-matrix-artifacts.json",
    "/tmp/matrices/oss/workspace-live-sync-matrix-artifacts.json",
  ])
  assert.deepEqual(distributedRuntimeMatrixArtifactIndexPathsFor({ runMatrixReports: false }), [])
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
})

test("matrix report child failures include generated evidence context", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-evidence-"))
  try {
    const scriptPath = path.join(rootDir, "failing-matrix.mjs")
    await writeFailingScript(scriptPath, "matrix failed")

    await assert.rejects(
      runDistributedRuntimeMatrixReportCommand({
        artifactIndexFlag: "--artifact-index",
        args: ["--label", "Alice's matrix run", "--include-hetzner"],
        cwd: rootDir,
        matrix: "remote-agent-runtime-matrix",
        outputDir: path.join(rootDir, "out"),
        reportFileName: "matrix.json",
        repo: "oss",
        scriptPath,
      }),
      (error) => {
        assert.match(error.message, /matrix report failed:/)
        assert.match(error.message, new RegExp(`cwd: ${escapeRegExp(rootDir)}`))
        assert.match(error.message, /repo: oss/)
        assert.match(error.message, /matrix: remote-agent-runtime-matrix/)
        assert.match(error.message, /--label 'Alice'\\''s matrix run'/)
        assert.match(error.message, /args: .*--include-hetzner .*--report .*matrix\.json .*--artifact-index .*matrix-artifacts\.json/)
        assert.match(error.message, /report: .*matrix\.json/)
        assert.match(error.message, /artifact-index: .*matrix-artifacts\.json/)
        assert.match(error.message, /stderr:\nmatrix failed/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("validation suite child failures include generated evidence context", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-evidence-"))
  try {
    const scriptPath = path.join(rootDir, "failing-suite.mjs")
    await writeFailingScript(scriptPath, "suite failed")

    await assert.rejects(
      runDistributedRuntimeValidationSuiteCommand({
        cwd: rootDir,
        outputDir: path.join(rootDir, "out"),
        preserveFailureRoot: path.join(rootDir, "out", "failed-run"),
        reportFileName: "suite.json",
        scriptPath,
      }),
      (error) => {
        assert.match(error.message, /validation suite failed:/)
        assert.match(error.message, new RegExp(`cwd: ${escapeRegExp(rootDir)}`))
        assert.match(error.message, /validation-suite: suite/)
        assert.match(error.message, /args: .*--run-json .*--output .*suite\.json .*--output-artifact-index .*arroba-drill-artifacts\.json .*--preserve-failure-root .*failed-run/)
        assert.match(error.message, /report: .*suite\.json/)
        assert.match(error.message, /artifact-index: .*arroba-drill-artifacts\.json/)
        assert.match(error.message, /failure-root: .*failed-run/)
        assert.match(error.message, /stderr:\nsuite failed/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeFailingScript(file, message) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `#!/usr/bin/env node
console.error(${JSON.stringify(message)})
process.exit(42)
`, "utf8")
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}
