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
      roots: [...generatedMatrixRoots].map((item) => path.resolve(item)).sort(),
      dryRun: options.matrixDryRun === true,
      continueOnFailure: options.matrixContinueOnFailure === true,
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
        }).map(distributedRuntimeMatrixCommandSummary)
        : [],
    },
    validationSuites: {
      enabled: options.runValidationSuites === true,
      artifactIndexes: [...validationSuiteArtifactIndexes].map((item) => path.resolve(item)).sort(),
      outputRoots: options.runValidationSuites === true
        ? [
          distributedRuntimeValidationSuiteOutputDirFor(options, "cloud"),
          distributedRuntimeValidationSuiteOutputDirFor(options, "oss"),
        ].map((item) => path.resolve(item)).sort()
        : [],
    },
  }
}

export function distributedRuntimeMatrixCommandsFor({
  cloudOutputDir,
  cloudRoot,
  commonArgs = [],
  ossOutputDir,
  ossRoot,
}) {
  return [
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, "--include-hetzner"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "native-provider-tui-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-native-provider-tui-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, "--include-hetzner", "--include-hosted-cloud"],
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
      args: [...commonArgs, "--include-self-hosted-relay"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "slice-runtime-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-slice-runtime-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--artifact-index",
      args: [...commonArgs, "--include-remote", "--include-hetzner", "--include-opencode"],
      cwd: ossRoot,
      outputDir: ossOutputDir,
      reportFileName: "workspace-live-sync-matrix.json",
      scriptPath: path.join(ossRoot, "apps", "cli", "scripts", "live-workspace-live-sync-matrix-drill.mjs"),
    },
    {
      artifactIndexFlag: "--output-artifact-index",
      args: [...commonArgs, "--include-hosted-cloud", "--include-vault"],
      cwd: cloudRoot,
      outputDir: cloudOutputDir,
      reportFileName: "cloud-slice-runtime-matrix.json",
      scriptPath: path.join(cloudRoot, "scripts", "staging-slice-runtime-matrix.mjs"),
    },
  ]
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
  try {
    await execFile(process.execPath, [
      scriptPath,
      ...args,
      "--report",
      reportPath,
      artifactIndexFlag,
      artifactIndexPath,
    ], { cwd, maxBuffer: 1024 * 1024 * 20 })
  } catch (error) {
    throw new Error(`matrix report failed: ${scriptPath}${childProcessOutputFor(error)}`)
  }
  return reportPath
}

export async function runDistributedRuntimeValidationSuitesFor(options) {
  if (!options.runValidationSuites) return []
  const ossOutputDir = distributedRuntimeValidationSuiteOutputDirFor(options, "oss")
  const cloudOutputDir = distributedRuntimeValidationSuiteOutputDirFor(options, "cloud")
  const ossArtifactIndex = await runDistributedRuntimeValidationSuiteCommand({
    cwd: options.ossRoot,
    outputDir: ossOutputDir,
    reportFileName: "drill-validation-suite-run.json",
    scriptPath: path.join(options.ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
  })
  const cloudArtifactIndex = await runDistributedRuntimeValidationSuiteCommand({
    cwd: options.cloudRoot,
    outputDir: cloudOutputDir,
    reportFileName: "cloud-validation-suite-run.json",
    scriptPath: path.join(options.cloudRoot, "scripts", "cloud-validation-suite.mjs"),
  })
  return [ossArtifactIndex, cloudArtifactIndex]
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
  reportFileName,
  scriptPath,
}) {
  const outputPath = path.join(outputDir, reportFileName)
  const artifactIndexPath = path.join(outputDir, "arroba-drill-artifacts.json")
  try {
    await execFile(process.execPath, [
      scriptPath,
      "--run-json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ], { cwd, maxBuffer: 1024 * 1024 * 20 })
  } catch (error) {
    throw new Error(`validation suite failed: ${scriptPath}${childProcessOutputFor(error)}`)
  }
  return artifactIndexPath
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
