#!/usr/bin/env node
import path from "node:path"
import { fileURLToPath } from "node:url"

import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./lib/drill-validation-gate-args.mjs"
import {
  DRILL_VALIDATION_GATE_PRESETS,
  drillValidationGateExitCode,
  formatDrillValidationGateSummary,
  runDrillValidationGate,
} from "./lib/drill-validation-gate.mjs"
import { diagnosticMetadataForValidationGateReport } from "./lib/drill-validation-gate-runtime-signal-metadata.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultOssRoot = path.resolve(scriptDir, "..", "..", "..")
const defaultCloudRoot = path.resolve(defaultOssRoot, "..", "arroba-cloud")

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-cross-repo-validation-gate.mjs [options]",
    "",
    "Verifies validation-platform evidence collected across arroba and arroba-cloud.",
    "By default it discovers matrix reports under both repos' .artifacts/drill-matrices roots.",
    "",
    "Options:",
    "  --oss-root DIR         OSS repo root; defaults to this script's repo root",
    "  --cloud-root DIR       Cloud repo root; defaults to ../arroba-cloud",
    "  --no-default-roots     Only use matrix roots passed explicitly with --matrix-root",
    "  --include-default-artifacts",
    "                         Discover artifact indexes under each repo's .artifacts root",
    "  --include-default-failures",
    "                         Discover failure manifests under each repo's .artifacts root",
    "  --platform-bundle DIR  Verify a drill platform bundle directory",
    "  --matrix-root ROOT     Discover matrix reports below ROOT; repeatable",
    "  --artifact-root ROOT   Discover artifact indexes below ROOT; repeatable",
    "  --failure-root ROOT    Discover failure manifests below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
    "  --require-complete     Fail when matrix reports include skipped/dry-run scenarios or unresolved exit criteria",
    "  --preset NAME[,NAME]   Apply named requirement preset; repeatable",
    `                         Known: ${Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort().join(", ")}`,
    "  --require-platform-coverage-area ID[,ID]",
    "  --require-artifact-schema SCHEMA[,SCHEMA]",
    "  --require-runtime-signal ID[,ID]",
    "  --require-failure-classification KIND[,KIND]",
    "  --require-matrix NAME[,NAME]",
    "  --require-matrix-classification KIND[,KIND]",
    "  --require-matrix-runtime-signal ID[,ID]",
    "  --require-deployment-preset NAME[,NAME]",
    "  --require-provider NAME[,NAME]",
    "  --require-scenario ID[,ID]",
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
  const gateOptions = gateOptionsFor(options)
  const report = await runDrillValidationGate(gateOptions)
  if (options.outputPath) {
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: report,
      metadata: {
        drill: "cross-repo-validation-gate",
        status: report.status,
        ossRoot: options.ossRoot,
        cloudRoot: options.cloudRoot,
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
    artifactRoots: [],
    cloudRoot: defaultCloudRoot,
    defaultRoots: true,
    includeDefaultArtifacts: false,
    includeDefaultFailures: false,
    failureRoots: [],
    help: false,
    json: false,
    matrixRoots: [],
    maxDepth: 8,
    ossRoot: defaultOssRoot,
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
    else if (arg === "--no-default-roots") options.defaultRoots = false
    else if (arg === "--include-default-artifacts") options.includeDefaultArtifacts = true
    else if (arg === "--include-default-failures") options.includeDefaultFailures = true
    else if (arg === "--require-complete") options.requireComplete = true
    else {
      const requirementIndex = parseValidationGateRequirementArg(argv, index, options)
      if (requirementIndex !== null) {
        index = requirementIndex
        continue
      }
      if (arg === "--oss-root") {
        options.ossRoot = path.resolve(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--oss-root=")) {
        options.ossRoot = path.resolve(arg.slice("--oss-root=".length))
      } else if (arg === "--cloud-root") {
        options.cloudRoot = path.resolve(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--cloud-root=")) {
        options.cloudRoot = path.resolve(arg.slice("--cloud-root=".length))
      } else if (arg === "--platform-bundle") {
        options.platformBundleDir = readValue(argv, index, arg)
        index += 1
      } else if (arg.startsWith("--platform-bundle=")) {
        options.platformBundleDir = arg.slice("--platform-bundle=".length)
      } else if (arg === "--matrix-root") {
        options.matrixRoots.push(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--matrix-root=")) {
        options.matrixRoots.push(arg.slice("--matrix-root=".length))
      } else if (arg === "--artifact-root") {
        options.artifactRoots.push(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--artifact-root=")) {
        options.artifactRoots.push(arg.slice("--artifact-root=".length))
      } else if (arg === "--failure-root") {
        options.failureRoots.push(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--failure-root=")) {
        options.failureRoots.push(arg.slice("--failure-root=".length))
      } else if (arg === "--max-depth") {
        options.maxDepth = parseDrillMaxDepth(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--max-depth=")) {
        options.maxDepth = parseDrillMaxDepth(arg.slice("--max-depth=".length))
      } else if (arg === "--output") {
        options.outputPath = readValue(argv, index, arg)
        index += 1
      } else if (arg.startsWith("--output=")) {
        options.outputPath = arg.slice("--output=".length)
      } else if (arg === "--output-artifact-index") {
        options.outputArtifactIndexPath = readValue(argv, index, arg)
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

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function gateOptionsFor(options) {
  const matrixRoots = [...options.matrixRoots]
  const artifactRoots = [...options.artifactRoots]
  const failureRoots = [...options.failureRoots]
  if (options.defaultRoots) {
    matrixRoots.push(
      path.join(options.ossRoot, ".artifacts", "drill-matrices"),
      path.join(options.cloudRoot, ".artifacts", "drill-matrices"),
    )
  }
  if (options.includeDefaultArtifacts) {
    artifactRoots.push(
      path.join(options.ossRoot, ".artifacts"),
      path.join(options.cloudRoot, ".artifacts"),
    )
  }
  if (options.includeDefaultFailures) {
    failureRoots.push(
      path.join(options.ossRoot, ".artifacts"),
      path.join(options.cloudRoot, ".artifacts"),
    )
  }
  return {
    artifactRoots: uniqueSortedResolvedPaths(artifactRoots),
    failureRoots: uniqueSortedResolvedPaths(failureRoots),
    matrixRoots: uniqueSortedResolvedPaths(matrixRoots),
    maxDepth: options.maxDepth,
    platformBundleDir: options.platformBundleDir,
    presets: options.presets,
    requireComplete: options.requireComplete,
    requiredPlatformCoverageAreas: options.requiredPlatformCoverageAreas,
    requiredArtifactSchemas: options.requiredArtifactSchemas,
    requiredRuntimeSignals: options.requiredRuntimeSignals,
    requiredFailureClassifications: options.requiredFailureClassifications,
    requiredMatrices: options.requiredMatrices,
    requiredMatrixClassifications: options.requiredMatrixClassifications,
    requiredMatrixRuntimeSignals: options.requiredMatrixRuntimeSignals,
    requiredDeploymentPresets: options.requiredDeploymentPresets,
    requiredProviders: options.requiredProviders,
    requiredScenarios: options.requiredScenarios,
  }
}

function uniqueSortedResolvedPaths(paths) {
  return [...new Set(paths.map((item) => path.resolve(item)))].sort()
}

main().catch((error) => {
  console.error(`[drill-cross-repo-validation-gate] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
