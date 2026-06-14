#!/usr/bin/env node
import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  drillValidationGateAggregateExitCode,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  readDrillValidationGateReport,
  summarizeDrillValidationGateReports,
} from "./lib/drill-validation-gate.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-gate-summary.mjs [options]",
    "",
    "Aggregates persisted drill validation gate reports.",
    "",
    "Options:",
    "  --gate-report PATH     Read a specific validation gate report; repeatable",
    "  --gate-root ROOT       Discover validation gate reports below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
    "  --require-preset NAME  Require aggregate evidence for a validation gate preset; repeatable or comma-separated",
    "  --require-platform-coverage-area AREA",
    "                         Require aggregate evidence for platform-bundle coverage areas",
    "  --require-failure-classification CLASSIFICATION",
    "                         Require aggregate evidence for failure taxonomy classifications",
    "  --require-matrix NAME  Require aggregate evidence for matrix names",
    "  --require-matrix-classification CLASSIFICATION",
    "                         Require aggregate evidence for matrix scenario classifications",
    "  --require-deployment-preset PRESET",
    "                         Require aggregate evidence for deployment presets",
    "  --require-provider PROVIDER",
    "                         Require aggregate evidence for provider profiles",
    "  --require-scenario ID  Require aggregate evidence for scenario ids",
    "  --json                 Print aggregate JSON",
    "  --output PATH          Write aggregate JSON to PATH",
    "  --output-artifact-index PATH",
    "                         Write an artifact index for --output",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const discovered = options.gateRoots.length > 0
    ? await findDrillValidationGateReportPaths(options.gateRoots, { maxDepth: options.maxDepth })
    : []
  const reportPaths = [...new Set([...options.gateReports, ...discovered])].sort()
  if (reportPaths.length === 0) {
    throw new Error("no validation gate reports found")
  }
  const reports = await Promise.all(reportPaths.map((reportPath) => readDrillValidationGateReport(reportPath)))
  const aggregate = summarizeDrillValidationGateReports(reports, {
    sources: reportPaths,
    requiredPresets: options.requiredPresets,
    requiredPlatformCoverageAreas: options.requiredPlatformCoverageAreas,
    requiredFailureClassifications: options.requiredFailureClassifications,
    requiredMatrices: options.requiredMatrices,
    requiredMatrixClassifications: options.requiredMatrixClassifications,
    requiredDeploymentPresets: options.requiredDeploymentPresets,
    requiredProviders: options.requiredProviders,
    requiredScenarios: options.requiredScenarios,
  })
  if (options.outputPath) {
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: aggregate,
      metadata: {
        drill: "validation-gate-summary",
        status: aggregate.status,
      },
    })
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
  } else {
    console.log(formatDrillValidationGateAggregateSummary(aggregate))
  }
  process.exitCode = drillValidationGateAggregateExitCode(aggregate)
}

function parseArgs(argv) {
  const options = {
    gateReports: [],
    gateRoots: [],
    help: false,
    json: false,
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    requiredDeploymentPresets: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredPlatformCoverageAreas: [],
    requiredProviders: [],
    requiredPresets: [],
    requiredScenarios: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--gate-report") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--gate-report requires a value")
      options.gateReports.push(value)
      index += 1
    } else if (arg.startsWith("--gate-report=")) {
      options.gateReports.push(arg.slice("--gate-report=".length))
    } else if (arg === "--gate-root") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--gate-root requires a value")
      options.gateRoots.push(value)
      index += 1
    } else if (arg.startsWith("--gate-root=")) {
      options.gateRoots.push(arg.slice("--gate-root=".length))
    } else if (arg === "--max-depth") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--max-depth requires a value")
      options.maxDepth = parseDrillMaxDepth(value)
      index += 1
    } else if (arg.startsWith("--max-depth=")) {
      options.maxDepth = parseDrillMaxDepth(arg.slice("--max-depth=".length))
    } else if (arg === "--require-preset") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-preset requires a value")
      options.requiredPresets.push(value)
      index += 1
    } else if (arg.startsWith("--require-preset=")) {
      options.requiredPresets.push(arg.slice("--require-preset=".length))
    } else if (arg === "--require-platform-coverage-area") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-platform-coverage-area requires a value")
      options.requiredPlatformCoverageAreas.push(value)
      index += 1
    } else if (arg.startsWith("--require-platform-coverage-area=")) {
      options.requiredPlatformCoverageAreas.push(arg.slice("--require-platform-coverage-area=".length))
    } else if (arg === "--require-failure-classification") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-failure-classification requires a value")
      options.requiredFailureClassifications.push(value)
      index += 1
    } else if (arg.startsWith("--require-failure-classification=")) {
      options.requiredFailureClassifications.push(arg.slice("--require-failure-classification=".length))
    } else if (arg === "--require-matrix") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-matrix requires a value")
      options.requiredMatrices.push(value)
      index += 1
    } else if (arg.startsWith("--require-matrix=")) {
      options.requiredMatrices.push(arg.slice("--require-matrix=".length))
    } else if (arg === "--require-matrix-classification") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-matrix-classification requires a value")
      options.requiredMatrixClassifications.push(value)
      index += 1
    } else if (arg.startsWith("--require-matrix-classification=")) {
      options.requiredMatrixClassifications.push(arg.slice("--require-matrix-classification=".length))
    } else if (arg === "--require-deployment-preset") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-deployment-preset requires a value")
      options.requiredDeploymentPresets.push(value)
      index += 1
    } else if (arg.startsWith("--require-deployment-preset=")) {
      options.requiredDeploymentPresets.push(arg.slice("--require-deployment-preset=".length))
    } else if (arg === "--require-provider") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-provider requires a value")
      options.requiredProviders.push(value)
      index += 1
    } else if (arg.startsWith("--require-provider=")) {
      options.requiredProviders.push(arg.slice("--require-provider=".length))
    } else if (arg === "--require-scenario") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-scenario requires a value")
      options.requiredScenarios.push(value)
      index += 1
    } else if (arg.startsWith("--require-scenario=")) {
      options.requiredScenarios.push(arg.slice("--require-scenario=".length))
    } else if (arg === "--output") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output requires a value")
      options.outputPath = value
      index += 1
    } else if (arg.startsWith("--output=")) {
      options.outputPath = arg.slice("--output=".length)
    } else if (arg === "--output-artifact-index") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output-artifact-index requires a value")
      options.outputArtifactIndexPath = value
      index += 1
    } else if (arg.startsWith("--output-artifact-index=")) {
      options.outputArtifactIndexPath = arg.slice("--output-artifact-index=".length)
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown argument: ${arg}`)
    } else {
      throw new Error(`unexpected argument: ${arg}`)
    }
  }
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-validation-gate-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
