import { execFile as execFileWithCallback } from "node:child_process"
import path from "node:path"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)

export async function runDistributedRuntimeMatrixReportsFor(options) {
  if (!options.runMatrixReports) return []
  const ossOutputDir = distributedRuntimeMatrixOutputDirFor(options, "oss")
  const cloudOutputDir = distributedRuntimeMatrixOutputDirFor(options, "cloud")
  const commonArgs = [
    ...(options.matrixDryRun ? ["--dry-run"] : []),
    ...(options.matrixContinueOnFailure ? ["--continue-on-failure"] : []),
  ]
  const matrixCommands = distributedRuntimeMatrixCommandsFor({
    cloudOutputDir,
    cloudRoot: options.cloudRoot,
    commonArgs,
    ossOutputDir,
    ossRoot: options.ossRoot,
    providerAccounts: options.providerAccounts,
  })
  for (const command of matrixCommands) {
    await runDistributedRuntimeMatrixReportCommand(command)
  }
  return [ossOutputDir, cloudOutputDir]
}

export function distributedRuntimeGeneratedEvidenceSummaryFor(options, {
  generatedMatrixRoots = [],
  validationSuiteArtifactIndexes = [],
} = {}) {
  return {
    matrixReports: {
      enabled: options.runMatrixReports === true,
      artifactIndexes: distributedRuntimeMatrixArtifactIndexPathsFor(options),
      roots: [...generatedMatrixRoots].map((item) => path.resolve(item)).sort(),
      dryRun: options.matrixDryRun === true,
      continueOnFailure: options.matrixContinueOnFailure === true,
      limitations: distributedRuntimeGeneratedMatrixLimitationsFor(options),
      commands: options.runMatrixReports === true
        ? distributedRuntimeMatrixCommandsFor({
          cloudOutputDir: distributedRuntimeMatrixOutputDirFor(options, "cloud"),
          cloudRoot: options.cloudRoot,
          commonArgs: [
            ...(options.matrixDryRun ? ["--dry-run"] : []),
            ...(options.matrixContinueOnFailure ? ["--continue-on-failure"] : []),
          ],
          ossOutputDir: distributedRuntimeMatrixOutputDirFor(options, "oss"),
          ossRoot: options.ossRoot,
          providerAccounts: options.providerAccounts,
        }).map(distributedRuntimeMatrixCommandSummary)
        : [],
    },
    validationSuites: {
      enabled: options.runValidationSuites === true,
      artifactIndexes: [...validationSuiteArtifactIndexes].map((item) => path.resolve(item)).sort(),
      failureRoots: options.runValidationSuites === true
        ? distributedRuntimeValidationSuiteFailureRootsFor(options)
        : [],
      commands: options.runValidationSuites === true
        ? distributedRuntimeValidationSuiteCommandsFor({
          cloudOutputDir: distributedRuntimeValidationSuiteOutputDirFor(options, "cloud"),
          cloudRoot: options.cloudRoot,
          ossOutputDir: distributedRuntimeValidationSuiteOutputDirFor(options, "oss"),
          ossRoot: options.ossRoot,
        }).map(distributedRuntimeValidationSuiteCommandSummary)
        : [],
      outputRoots: options.runValidationSuites === true
        ? [
          distributedRuntimeValidationSuiteOutputDirFor(options, "cloud"),
          distributedRuntimeValidationSuiteOutputDirFor(options, "oss"),
        ].map((item) => path.resolve(item)).sort()
        : [],
    },
  }
}

export function distributedRuntimeMatrixArtifactIndexPathsFor(options) {
  if (options.runMatrixReports !== true) return []
  return distributedRuntimeMatrixCommandsFor({
    cloudOutputDir: distributedRuntimeMatrixOutputDirFor(options, "cloud"),
    cloudRoot: options.cloudRoot,
    commonArgs: [
      ...(options.matrixDryRun ? ["--dry-run"] : []),
      ...(options.matrixContinueOnFailure ? ["--continue-on-failure"] : []),
    ],
    ossOutputDir: distributedRuntimeMatrixOutputDirFor(options, "oss"),
    ossRoot: options.ossRoot,
    providerAccounts: options.providerAccounts,
  })
    .map((command) => distributedRuntimeMatrixCommandSummary(command).artifactIndexPath)
    .map((item) => path.resolve(item))
    .sort()
}

export function distributedRuntimeValidationSuiteFailureRootsFor(options) {
  return distributedRuntimeValidationSuiteCommandsFor({
    cloudOutputDir: distributedRuntimeValidationSuiteOutputDirFor(options, "cloud"),
    cloudRoot: options.cloudRoot,
    ossOutputDir: distributedRuntimeValidationSuiteOutputDirFor(options, "oss"),
    ossRoot: options.ossRoot,
  })
    .map((command) => command.preserveFailureRoot)
    .filter((failureRoot) => typeof failureRoot === "string" && failureRoot.length > 0)
    .map((failureRoot) => path.resolve(failureRoot))
    .sort()
}

export function distributedRuntimeGeneratedMatrixLimitationsFor(options) {
  if (options.runMatrixReports !== true || options.matrixDryRun !== true) return []
  return [{
    kind: "dry-run-classification-coverage",
    owner: "validation-harness",
    nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before treating required matrix classifications as release evidence",
  }]
}

export function distributedRuntimeMatrixCommandsFor({
  cloudOutputDir,
  cloudRoot,
  commonArgs = [],
  ossOutputDir,
  ossRoot,
  providerAccounts = {},
}) {
  return [
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, ...providerAccountArgsFor(providerAccounts, ["claude", "codex", "opencode"]), "--include-hetzner"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "native-provider-tui-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, ...providerAccountArgsFor(providerAccounts, ["claude", "codex", "opencode"]), "--include-hetzner", "--include-hosted-cloud"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "remote-agent-runtime-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-remote-agent-runtime-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, "--include-hetzner"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "remote-home-extension-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-remote-home-extension-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, ...providerAccountArgsFor(providerAccounts, ["claude", "codex", "opencode"]), "--include-self-hosted-relay"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "slice-runtime-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-slice-runtime-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, ...providerAccountArgsFor(providerAccounts, ["codex", "opencode"]), "--include-remote", "--include-hetzner", "--include-opencode"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "workspace-live-sync-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-workspace-live-sync-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--output-artifact-index",
      args: [...commonArgs, ...providerAccountArgsFor(providerAccounts, ["claude", "codex", "opencode"]), "--include-hosted-cloud", "--include-vault"],
      cwd: cloudRoot,
      outputDir: cloudOutputDir,
      reportFileName: "cloud-slice-runtime-matrix.json",
      scriptPath: path.join(cloudRoot, "scripts", "staging-slice-runtime-matrix.mjs"),
    },
  ]
}

function providerAccountArgsFor(providerAccounts = {}, supportedProviders = []) {
  const supported = new Set(supportedProviders)
  return Object.keys(providerAccounts)
    .filter((provider) => supported.has(provider))
    .sort()
    .flatMap((provider) => ["--provider-account", `${provider}=${providerAccounts[provider]}`])
}

export function distributedRuntimeMatrixOutputDirFor(options, repo) {
  if (options.matrixOutputRoot) {
    return path.join(options.matrixOutputRoot, repo)
  }
  if (repo === "oss") {
    return path.join(options.ossRoot, ".artifacts", "drill-matrices", "distributed-runtime-gate")
  }
  return path.join(options.cloudRoot, ".artifacts", "drill-matrices", "distributed-runtime-gate")
}

export function distributedRuntimeMatrixCommandSummary(command) {
  return {
    artifactIndexPath: path.join(command.outputDir, `${path.basename(command.reportFileName, ".json")}-artifacts.json`),
    args: [...command.args],
    cwd: command.cwd,
    reportPath: path.join(command.outputDir, command.reportFileName),
    scriptPath: command.scriptPath,
  }
}

export async function runDistributedRuntimeMatrixReportCommand({
  artifactIndexFlag,
  args,
  cwd,
  outputDir,
  reportFileName,
  scriptPath,
}) {
  const reportPath = path.join(outputDir, reportFileName)
  const artifactIndexPath = path.join(outputDir, `${path.basename(reportFileName, ".json")}-artifacts.json`)
  const commandArgs = [
    scriptPath,
    ...args,
    "--report",
    reportPath,
    artifactIndexFlag,
    artifactIndexPath,
  ]
  try {
    await execFile(process.execPath, commandArgs, { cwd, maxBuffer: 1024 * 1024 * 20 })
  } catch (error) {
    throw new Error(`matrix report failed: ${scriptPath}${childCommandContextFor({
      args: commandArgs,
      artifactIndexPath,
      cwd,
      reportPath,
    })}${childProcessOutputFor(error)}`)
  }
  return reportPath
}

export async function runDistributedRuntimeValidationSuitesFor(options) {
  if (!options.runValidationSuites) return []
  const ossOutputDir = distributedRuntimeValidationSuiteOutputDirFor(options, "oss")
  const cloudOutputDir = distributedRuntimeValidationSuiteOutputDirFor(options, "cloud")
  const artifactIndexes = []
  for (const command of distributedRuntimeValidationSuiteCommandsFor({
    cloudOutputDir,
    cloudRoot: options.cloudRoot,
    ossOutputDir,
    ossRoot: options.ossRoot,
  })) {
    artifactIndexes.push(await runDistributedRuntimeValidationSuiteCommand(command))
  }
  return artifactIndexes
}

export function distributedRuntimeValidationSuiteOutputDirFor(options, repo) {
  if (options.validationSuiteOutputRoot) {
    return path.join(options.validationSuiteOutputRoot, repo)
  }
  if (repo === "oss") {
    return path.join(options.ossRoot, ".artifacts", "validation-suite", "distributed-runtime-gate")
  }
  return path.join(options.cloudRoot, ".artifacts", "validation-suite", "distributed-runtime-gate")
}

export async function runDistributedRuntimeValidationSuiteCommand({
  cwd,
  outputDir,
  preserveFailureRoot = null,
  reportFileName,
  scriptPath,
}) {
  const outputPath = path.join(outputDir, reportFileName)
  const artifactIndexPath = path.join(outputDir, "arroba-drill-artifacts.json")
  const args = [
    scriptPath,
    "--run-json",
    "--output",
    outputPath,
    "--output-artifact-index",
    artifactIndexPath,
    ...(preserveFailureRoot ? ["--preserve-failure-root", preserveFailureRoot] : []),
  ]
  try {
    await execFile(process.execPath, args, { cwd, maxBuffer: 1024 * 1024 * 20 })
  } catch (error) {
    throw new Error(`validation suite failed: ${scriptPath}${childCommandContextFor({
      args,
      artifactIndexPath,
      cwd,
      failureRoot: preserveFailureRoot,
      reportPath: outputPath,
    })}${childProcessOutputFor(error)}`)
  }
  return artifactIndexPath
}

export function distributedRuntimeValidationSuiteCommandsFor({
  cloudOutputDir,
  cloudRoot,
  ossOutputDir,
  ossRoot,
}) {
  return [
    {
      cwd: ossRoot,
      outputDir: ossOutputDir,
      preserveFailureRoot: path.join(ossOutputDir, "failed-run"),
      reportFileName: "drill-validation-suite-run.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    },
    {
      cwd: cloudRoot,
      outputDir: cloudOutputDir,
      preserveFailureRoot: path.join(cloudOutputDir, "failed-run"),
      reportFileName: "cloud-validation-suite-run.json",
      scriptPath: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    },
  ]
}

export function distributedRuntimeValidationSuiteCommandSummary(command) {
  return {
    artifactIndexPath: path.join(command.outputDir, "arroba-drill-artifacts.json"),
    args: ["--run-json", "--preserve-failure-root", command.preserveFailureRoot],
    cwd: command.cwd,
    failureRoot: command.preserveFailureRoot,
    reportPath: path.join(command.outputDir, command.reportFileName),
    scriptPath: command.scriptPath,
  }
}

function childCommandContextFor({
  args,
  artifactIndexPath,
  cwd,
  failureRoot = null,
  reportPath,
}) {
  return [
    `\ncwd: ${cwd}`,
    `\nargs: ${args.join(" ")}`,
    `\nreport: ${reportPath}`,
    `\nartifact-index: ${artifactIndexPath}`,
    ...(failureRoot ? [`\nfailure-root: ${failureRoot}`] : []),
  ].join("")
}

function childProcessOutputFor(error) {
  const stderr = typeof error.stderr === "string" && error.stderr.trim().length > 0
    ? `\nstderr:\n${error.stderr.trim()}`
    : ""
  const stdout = typeof error.stdout === "string" && error.stdout.trim().length > 0
    ? `\nstdout:\n${error.stdout.trim()}`
    : ""
  return `${stderr}${stdout}`
}
