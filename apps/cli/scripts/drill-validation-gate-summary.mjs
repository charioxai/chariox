#!/usr/bin/env node
import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./lib/drill-validation-gate-args.mjs"
import {
  drillValidationGateAggregateExitCode,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  readDrillValidationGateReport,
  summarizeDrillValidationGateReports,
} from "./lib/drill-validation-gate.mjs"
import { diagnosticMetadataForValidationGateAggregate } from "./lib/drill-validation-gate-runtime-signal-metadata.mjs"

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
    "  --require-artifact-coverage-area AREA",
    "                         Require aggregate evidence for artifact metadata coverage areas",
    "  --require-artifact-schema SCHEMA",
    "                         Require aggregate evidence for artifact schemas",
    "  --require-artifact-kind KIND",
    "                         Require aggregate evidence for artifact kinds",
    "  --require-artifact-evidence-repo REPO",
    "                         Require aggregate evidence from source repos",
    "  --require-runtime-signal ID",
    "                         Require aggregate evidence for platform runtime signals",
    "  --require-failure-classification CLASSIFICATION",
    "                         Require aggregate evidence for failure taxonomy classifications",
    "  --require-matrix NAME  Require aggregate evidence for matrix names",
    "  --require-matrix-classification CLASSIFICATION",
    "                         Require aggregate evidence for matrix scenario classifications",
    "  --require-matrix-runtime-signal ID",
    "                         Require aggregate evidence for matrix runtime signals",
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
    requiredArtifactCoverageAreas: options.requiredArtifactCoverageAreas,
    requiredArtifactSchemas: options.requiredArtifactSchemas,
    requiredArtifactKinds: options.requiredArtifactKinds,
    requiredArtifactEvidenceRepos: options.requiredArtifactEvidenceRepos,
    requiredRuntimeSignals: options.requiredRuntimeSignals,
    requiredFailureClassifications: options.requiredFailureClassifications,
    requiredMatrices: options.requiredMatrices,
    requiredMatrixClassifications: options.requiredMatrixClassifications,
    requiredMatrixRuntimeSignals: options.requiredMatrixRuntimeSignals,
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
        ...diagnosticMetadataForValidationGateAggregate(aggregate),
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
    ...validationGateRequirementOptionDefaults({ presetKey: "requiredPresets" }),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    const requirementIndex = parseValidationGateRequirementArg(argv, index, options, {
      presetFlag: "--require-preset",
      presetKey: "requiredPresets",
    })
    if (requirementIndex !== null) {
      index = requirementIndex
      continue
    }
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
