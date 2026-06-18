#!/usr/bin/env node
import {
  parseDrillMaxDepth,
  parseDrillNonNegativeInteger,
} from "./lib/drill-cli-args.mjs"
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
  runDrillValidationGate,
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
    "  --artifact-index PATH  Read a specific artifact index as aggregate artifact metadata evidence; repeatable",
    "  --artifact-root ROOT   Discover artifact indexes below ROOT as aggregate artifact metadata evidence; repeatable",
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
    "  --require-artifact-generated-evidence-kind KIND",
    "                         Require aggregate evidence for artifact generated evidence kinds",
    "  --require-artifact-generated-matrix-artifact-index PATH",
    "                         Require aggregate artifact metadata for generated matrix artifact indexes",
    "  --require-artifact-generated-matrix-limitation KIND",
    "                         Require aggregate evidence for artifact generated matrix limitations",
    "  --require-artifact-generated-matrix-name NAME",
    "                         Require aggregate evidence for artifact generated matrix names",
    "  --require-artifact-generated-matrix-repo REPO",
    "                         Require aggregate evidence for artifact generated matrix repos",
    "  --require-artifact-evidence-repo REPO",
    "                         Require aggregate evidence from source repos",
    "  --require-artifact-provider-account-alias P=A",
    "                         Require aggregate artifact metadata for provider account alias labels",
    "  --require-artifact-validation-preset NAME",
    "                         Require aggregate artifact metadata for validation preset labels",
    "  --require-artifact-runtime-signal ID",
    "                         Require aggregate artifact metadata for runtime signals",
    "  --require-artifact-runtime-signal-owner OWNER",
    "                         Require aggregate artifact metadata for runtime signal owners",
    "  --require-artifact-owner OWNER",
    "                         Require aggregate artifact metadata from artifact owners",
  "  --require-artifact-classification CLASSIFICATION",
  "                         Require aggregate artifact metadata for classifications",
  "  --require-artifact-failure-classification CLASSIFICATION",
  "                         Require aggregate artifact metadata for failure classifications",
  "  --require-artifact-planned-owner OWNER",
    "                         Require aggregate artifact metadata for planned next-action owners",
    "  --require-artifact-planned-classification CLASSIFICATION",
    "                         Require aggregate artifact metadata for planned next-action classifications",
    "  --require-artifact-exit-criterion-status STATUS",
    "                         Require aggregate artifact metadata for exit criterion statuses",
    "  --require-artifact-incomplete-exit-criterion-status STATUS",
    "                         Require aggregate artifact metadata for incomplete exit criterion statuses",
    "  --require-artifact-max-age-ms MS",
    "                         Fail when artifact metadata inputs include stale artifact indexes",
    "  --require-runtime-signal ID",
    "                         Require aggregate evidence for platform runtime signals",
    "  --require-runtime-signal-owner OWNER",
    "                         Require aggregate evidence for platform runtime signal owners",
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
    "  --require-generated-evidence-kind KIND",
    "                         Require generated validation-suite-run or matrix-report evidence",
    "  --require-generated-matrix-artifact-index PATH",
    "                         Require generated matrix artifact-index evidence",
    "  --require-generated-matrix-limitation KIND",
    "                         Require aggregate evidence for generated matrix limitations",
    "  --require-generated-validation-suite-artifact-index PATH",
    "                         Require generated validation-suite artifact-index evidence",
    "  --require-generated-validation-suite-failure-root PATH",
    "                         Require generated validation-suite failure-root evidence",
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
  const artifactCoverageReport = await artifactCoverageReportForSummary(options)
  const aggregate = summarizeDrillValidationGateReports(reports, {
    sources: reportPaths,
    supplementalArtifactReports: artifactCoverageReport ? [artifactCoverageReport.report] : [],
    supplementalArtifactSources: artifactCoverageReport ? [artifactCoverageReport.source] : [],
    requiredPresets: options.requiredPresets,
    requiredPlatformCoverageAreas: options.requiredPlatformCoverageAreas,
    requiredArtifactCoverageAreas: options.requiredArtifactCoverageAreas,
    requiredArtifactSchemas: options.requiredArtifactSchemas,
    requiredArtifactKinds: options.requiredArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds: options.requiredArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedMatrixArtifactIndexes: options.requiredArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: options.requiredArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: options.requiredArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: options.requiredArtifactGeneratedMatrixRepos,
    requiredArtifactGeneratedValidationSuiteFailureRoots: options.requiredArtifactGeneratedValidationSuiteFailureRoots,
    requiredArtifactEvidenceRepos: options.requiredArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: options.requiredArtifactProviderAccountAliases,
    requiredArtifactValidationPresets: options.requiredArtifactValidationPresets,
    requiredArtifactRuntimeSignals: options.requiredArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: options.requiredArtifactRuntimeSignalOwners,
    requiredArtifactOwners: options.requiredArtifactOwners,
    requiredArtifactClassifications: options.requiredArtifactClassifications,
    requiredArtifactFailureClassifications: options.requiredArtifactFailureClassifications,
    requiredArtifactExitCriterionStatuses: options.requiredArtifactExitCriterionStatuses,
    requiredArtifactIncompleteExitCriterionStatuses: options.requiredArtifactIncompleteExitCriterionStatuses,
    requiredRuntimeSignals: options.requiredRuntimeSignals,
    requiredRuntimeSignalOwners: options.requiredRuntimeSignalOwners,
    requiredFailureClassifications: options.requiredFailureClassifications,
    requiredMatrices: options.requiredMatrices,
    requiredMatrixClassifications: options.requiredMatrixClassifications,
    requiredMatrixRuntimeSignals: options.requiredMatrixRuntimeSignals,
    requiredDeploymentPresets: options.requiredDeploymentPresets,
    requiredProviders: options.requiredProviders,
    requiredScenarios: options.requiredScenarios,
    requiredGeneratedEvidenceKinds: options.requiredGeneratedEvidenceKinds,
    requiredGeneratedMatrixArtifactIndexes: options.requiredGeneratedMatrixArtifactIndexes,
    requiredGeneratedMatrixLimitations: options.requiredGeneratedMatrixLimitations,
    requiredGeneratedValidationSuiteArtifactIndexes: options.requiredGeneratedValidationSuiteArtifactIndexes,
    requiredGeneratedValidationSuiteFailureRoots: options.requiredGeneratedValidationSuiteFailureRoots,
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
    artifactIndexes: [],
    artifactRoots: [],
    gateReports: [],
    gateRoots: [],
    help: false,
    json: false,
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    requiredArtifactMaxAgeMs: null,
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
    else if (arg === "--artifact-index") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--artifact-index requires a value")
      options.artifactIndexes.push(value)
      index += 1
    } else if (arg.startsWith("--artifact-index=")) {
      options.artifactIndexes.push(arg.slice("--artifact-index=".length))
    } else if (arg === "--artifact-root") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--artifact-root requires a value")
      options.artifactRoots.push(value)
      index += 1
    } else if (arg.startsWith("--artifact-root=")) {
      options.artifactRoots.push(arg.slice("--artifact-root=".length))
    } else if (arg === "--gate-report") {
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
    } else if (arg === "--require-artifact-max-age-ms") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-artifact-max-age-ms requires a value")
      options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(value, "--require-artifact-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-artifact-max-age-ms=")) {
      options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(
        arg.slice("--require-artifact-max-age-ms=".length),
        "--require-artifact-max-age-ms",
      )
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

async function artifactCoverageReportForSummary(options) {
  if (options.artifactIndexes.length === 0
    && options.artifactRoots.length === 0
    && options.requiredArtifactMaxAgeMs === null) return null
  const report = await runDrillValidationGate({
    artifactIndexes: options.artifactIndexes,
    artifactRoots: options.artifactRoots,
    maxDepth: options.maxDepth,
    requiredArtifactCoverageAreas: options.requiredArtifactCoverageAreas,
    requiredArtifactSchemas: options.requiredArtifactSchemas,
    requiredArtifactKinds: options.requiredArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds: options.requiredArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedMatrixArtifactIndexes: options.requiredArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: options.requiredArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: options.requiredArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: options.requiredArtifactGeneratedMatrixRepos,
    requiredArtifactGeneratedValidationSuiteFailureRoots: options.requiredArtifactGeneratedValidationSuiteFailureRoots,
    requiredArtifactEvidenceRepos: options.requiredArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: options.requiredArtifactProviderAccountAliases,
    requiredArtifactRuntimeSignals: options.requiredArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: options.requiredArtifactRuntimeSignalOwners,
    requiredArtifactOwners: options.requiredArtifactOwners,
    requiredArtifactClassifications: options.requiredArtifactClassifications,
    requiredArtifactFailureClassifications: options.requiredArtifactFailureClassifications,
    requiredArtifactExitCriterionStatuses: options.requiredArtifactExitCriterionStatuses,
    requiredArtifactIncompleteExitCriterionStatuses: options.requiredArtifactIncompleteExitCriterionStatuses,
    requiredArtifactMaxAgeMs: options.requiredArtifactMaxAgeMs,
  })
  return {
    report,
    source: "artifact metadata inputs",
  }
}

main().catch((error) => {
  console.error(`[drill-validation-gate-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
