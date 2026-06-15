#!/usr/bin/env node
import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./lib/drill-validation-gate-args.mjs"
import {
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  drillValidationGateExitCode,
  formatDrillValidationGateSummary,
  runDrillValidationGate,
} from "./lib/drill-validation-gate.mjs"
import { diagnosticMetadataForValidationGateReport } from "./lib/drill-validation-gate-runtime-signal-metadata.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-gate.mjs [options]",
    "",
    "Verifies collected validation platform artifacts for CI or staging gates.",
    "",
    "Options:",
    "  --list-presets       List validation gate requirement presets and exit",
    "  --preset NAME[,NAME]  Apply named requirement preset; repeatable",
    `                         Known: ${Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort().join(", ")}`,
    "  --platform-bundle DIR  Verify a drill platform bundle directory",
    "  --artifact-index PATH  Read and verify a specific artifact index; repeatable",
    "  --artifact-root ROOT   Discover artifact indexes below ROOT; repeatable",
    "  --matrix-report PATH    Read a specific matrix report; repeatable",
    "  --matrix-root ROOT     Discover matrix reports below ROOT; repeatable",
    "  --failure-manifest PATH",
    "                         Read a specific failure manifest or preserved root; repeatable",
    "  --failure-root ROOT    Discover failure manifests below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
    "  --require-complete     Fail when matrix reports include skipped/dry-run scenarios or unresolved exit criteria",
    "  --require-platform-coverage-area ID[,ID]",
    "                         Fail when platform bundle validation suite lacks each coverage area; repeatable",
    "  --require-artifact-coverage-area ID[,ID]",
    "                         Fail when artifact index metadata lacks each coverage area; repeatable",
    "  --require-artifact-schema SCHEMA[,SCHEMA]",
    "                         Fail when artifact indexes do not include each artifact schema; repeatable",
    "  --require-artifact-kind KIND[,KIND]",
    "                         Fail when artifact index metadata lacks each artifact kind; repeatable",
  "  --require-artifact-generated-evidence-kind KIND[,KIND]",
  "                         Fail when artifact index metadata lacks each generated evidence kind; repeatable",
  "  --require-artifact-generated-matrix-limitation KIND[,KIND]",
  "                         Fail when artifact index metadata lacks each generated matrix limitation; repeatable",
  "  --require-artifact-evidence-repo REPO[,REPO]",
    "                         Fail when artifact index metadata lacks evidence from each repo; repeatable",
    "  --require-artifact-runtime-signal ID[,ID]",
    "                         Fail when artifact index metadata lacks each runtime signal; repeatable",
    "  --require-artifact-runtime-signal-owner OWNER[,OWNER]",
    "                         Fail when artifact index metadata lacks each runtime signal owner; repeatable",
    "  --require-artifact-owner OWNER[,OWNER]",
    "                         Fail when artifact index metadata lacks each owner; repeatable",
    "  --require-artifact-classification KIND[,KIND]",
    "                         Fail when artifact index metadata lacks each classification; repeatable",
    "  --require-artifact-exit-criterion-status STATUS[,STATUS]",
    "                         Fail when artifact index metadata lacks each exit criterion status; repeatable",
    "  --require-artifact-incomplete-exit-criterion-status STATUS[,STATUS]",
    "                         Fail when artifact index metadata lacks each incomplete exit criterion status; repeatable",
    "  --require-runtime-signal ID[,ID]",
    "                         Fail when platform bundle lacks each distributed runtime signal; repeatable",
    "  --require-failure-classification KIND[,KIND]",
    "                         Fail when platform bundle failure taxonomy lacks each classification; repeatable",
    "  --require-matrix NAME[,NAME]",
    "                         Fail when matrix reports do not include each matrix name; repeatable",
    "  --require-matrix-classification KIND[,KIND]",
    "                         Fail when matrix reports do not include each failure classification; repeatable",
    "  --require-matrix-runtime-signal ID[,ID]",
    "                         Fail when matrix reports do not include each runtime signal; repeatable",
    "  --require-deployment-preset NAME[,NAME]",
    "                         Fail when matrix reports do not cover each deployment preset; repeatable",
    "  --require-provider NAME[,NAME]",
    "                         Fail when matrix reports do not cover each provider; repeatable",
    "  --require-scenario ID[,ID]",
    "                         Fail when matrix reports do not include each scenario id; repeatable",
    "  --json                 Print gate report JSON",
    "  --output PATH          Write gate report JSON to PATH",
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
  if (options.listPresets) {
    const presets = describeDrillValidationGatePresets({ names: options.presets.length > 0 ? options.presets : null })
    if (options.json) {
      console.log(JSON.stringify({ presets }, null, 2))
    } else {
      console.log(formatPresetList(presets))
    }
    return
  }
  const report = await runDrillValidationGate(options)
  if (options.outputPath) {
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: report,
      metadata: {
        drill: "validation-gate",
        status: report.status,
        ...diagnosticMetadataForValidationGateReport(report),
      },
    })
  }
  if (options.json) {
    console.log(JSON.stringify(report, null, 2))
  } else {
    console.log(formatDrillValidationGateSummary(report))
  }
  process.exitCode = drillValidationGateExitCode(report)
}

function parseArgs(argv) {
  const options = {
    artifactIndexes: [],
    artifactRoots: [],
    failureRoots: [],
    failureInputs: [],
    help: false,
    json: false,
    listPresets: false,
    matrixReports: [],
    matrixRoots: [],
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    platformBundleDir: null,
    requireComplete: false,
    ...validationGateRequirementOptionDefaults({ presetKey: "presets" }),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--list-presets") options.listPresets = true
    else if (arg === "--require-complete") options.requireComplete = true
    else {
      const requirementIndex = parseValidationGateRequirementArg(argv, index, options)
      if (requirementIndex !== null) {
        index = requirementIndex
        continue
      }
      if (arg === "--artifact-index") {
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
      } else if (arg === "--platform-bundle") {
        const value = argv[index + 1]
        if (!value || value.startsWith("--")) throw new Error("--platform-bundle requires a value")
        options.platformBundleDir = value
        index += 1
      } else if (arg.startsWith("--platform-bundle=")) {
        options.platformBundleDir = arg.slice("--platform-bundle=".length)
      } else if (arg === "--matrix-report") {
        const value = argv[index + 1]
        if (!value || value.startsWith("--")) throw new Error("--matrix-report requires a value")
        options.matrixReports.push(value)
        index += 1
      } else if (arg.startsWith("--matrix-report=")) {
        options.matrixReports.push(arg.slice("--matrix-report=".length))
      } else if (arg === "--matrix-root") {
        const value = argv[index + 1]
        if (!value || value.startsWith("--")) throw new Error("--matrix-root requires a value")
        options.matrixRoots.push(value)
        index += 1
      } else if (arg.startsWith("--matrix-root=")) {
        options.matrixRoots.push(arg.slice("--matrix-root=".length))
      } else if (arg === "--failure-manifest") {
        const value = argv[index + 1]
        if (!value || value.startsWith("--")) throw new Error("--failure-manifest requires a value")
        options.failureInputs.push(value)
        index += 1
      } else if (arg.startsWith("--failure-manifest=")) {
        options.failureInputs.push(arg.slice("--failure-manifest=".length))
      } else if (arg === "--failure-root") {
        const value = argv[index + 1]
        if (!value || value.startsWith("--")) throw new Error("--failure-root requires a value")
        options.failureRoots.push(value)
        index += 1
      } else if (arg.startsWith("--failure-root=")) {
        options.failureRoots.push(arg.slice("--failure-root=".length))
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
  }
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  return options
}

function formatPresetList(presets) {
  const lines = ["validation gate presets:"]
  for (const preset of presets) {
    lines.push(`- ${preset.name}: ${preset.description}`)
    lines.push(`  platform_coverage=${preset.requiredPlatformCoverageAreas.join(",") || "none"}`)
    lines.push(`  runtime_signals=${preset.requiredRuntimeSignals.join(",") || "none"}`)
    lines.push(`  failure_classifications=${preset.requiredFailureClassifications.join(",") || "none"}`)
    lines.push(`  matrices=${preset.requiredMatrices.join(",") || "none"}`)
    lines.push(`  matrix_classifications=${preset.requiredMatrixClassifications.join(",") || "none"}`)
    lines.push(`  matrix_runtime_signals=${preset.requiredMatrixRuntimeSignals.join(",") || "none"}`)
  }
  return lines.join("\n")
}

main().catch((error) => {
  console.error(`[drill-validation-gate] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
