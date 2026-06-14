#!/usr/bin/env node
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./lib/drill-validation-gate-args.mjs"
import {
  drillValidationGateExitCode,
  formatDrillValidationGateSummary,
  runDrillValidationGate,
} from "./lib/drill-validation-gate.mjs"
import { diagnosticMetadataForValidationGateReport } from "./lib/drill-validation-gate-runtime-signal-metadata.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultOssRoot = path.resolve(scriptDir, "..", "..", "..")
const defaultCloudRoot = path.resolve(defaultOssRoot, "..", "arroba-cloud")

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-distributed-runtime-gate.mjs [options]",
    "",
    "Runs the release-level distributed-runtime validation gate across arroba and arroba-cloud matrix artifacts.",
    "This is a focused wrapper for --preset distributed-runtime; it generates a platform bundle unless one is supplied.",
    "",
    "Options:",
    "  --oss-root DIR          OSS repo root; defaults to this script's repo root",
    "  --cloud-root DIR        Cloud repo root; defaults to ../arroba-cloud",
    "  --no-default-roots      Only use matrix roots passed explicitly with --matrix-root",
    "  --matrix-root ROOT      Discover matrix reports below ROOT; repeatable",
    "  --include-default-artifacts",
    "                         Discover artifact indexes under each repo's .artifacts root",
    "  --include-default-failures",
    "                         Discover failure manifests under each repo's .artifacts root",
    "  --failure-root ROOT     Discover failure manifests below ROOT; repeatable",
    "  --platform-bundle DIR   Use an existing drill platform bundle instead of generating one",
    "  --max-depth N           Limit artifact discovery depth; defaults to 8",
    "  --require-complete      Fail when matrix reports include skipped/dry-run scenarios or unresolved exit criteria",
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
    "  --json                  Print gate report JSON",
    "  --output PATH           Write gate report JSON to PATH",
    "  --output-artifact-index PATH",
    "                          Write an artifact index for --output",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const generatedBundleDir = options.platformBundleDir ? null : await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-platform-"))
  try {
    const platformBundleDir = options.platformBundleDir ?? generatedBundleDir
    if (!options.platformBundleDir) {
      await writeDrillPlatformBundle(platformBundleDir)
    }
    const report = await runDrillValidationGate({
      artifactRoots: artifactRootsFor(options),
      failureRoots: failureRootsFor(options),
      matrixRoots: matrixRootsFor(options),
      maxDepth: options.maxDepth,
      platformBundleDir,
      presets: ["distributed-runtime"],
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
    })
    if (options.outputPath) {
      await writeDrillJsonArtifactOutput({
        outputPath: options.outputPath,
        artifactIndexPath: options.outputArtifactIndexPath,
        value: report,
        metadata: {
          drill: "distributed-runtime-gate",
          status: report.status,
          preset: "distributed-runtime",
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
  } finally {
    if (generatedBundleDir) {
      await rm(generatedBundleDir, { recursive: true, force: true }).catch(() => {})
    }
  }
}

function parseArgs(argv) {
  const options = {
    cloudRoot: defaultCloudRoot,
    defaultRoots: true,
    failureRoots: [],
    help: false,
    includeDefaultArtifacts: false,
    includeDefaultFailures: false,
    json: false,
    matrixRoots: [],
    maxDepth: 8,
    ossRoot: defaultOssRoot,
    outputArtifactIndexPath: null,
    outputPath: null,
    platformBundleDir: null,
    requireComplete: false,
    ...validationGateRequirementOptionDefaults(),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--no-default-roots") options.defaultRoots = false
    else if (arg === "--include-default-artifacts") options.includeDefaultArtifacts = true
    else if (arg === "--include-default-failures") options.includeDefaultFailures = true
    else if (arg === "--require-complete") options.requireComplete = true
    else if (arg === "--oss-root") {
      options.ossRoot = path.resolve(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--oss-root=")) {
      options.ossRoot = path.resolve(arg.slice("--oss-root=".length))
    } else if (arg === "--cloud-root") {
      options.cloudRoot = path.resolve(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--cloud-root=")) {
      options.cloudRoot = path.resolve(arg.slice("--cloud-root=".length))
    } else if (arg === "--matrix-root") {
      options.matrixRoots.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--matrix-root=")) {
      options.matrixRoots.push(arg.slice("--matrix-root=".length))
    } else if (arg === "--failure-root") {
      options.failureRoots.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--failure-root=")) {
      options.failureRoots.push(arg.slice("--failure-root=".length))
    } else if (arg === "--platform-bundle") {
      options.platformBundleDir = readValue(argv, index, arg)
      index += 1
    } else if (arg.startsWith("--platform-bundle=")) {
      options.platformBundleDir = arg.slice("--platform-bundle=".length)
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
      const requirementIndex = parseValidationGateRequirementArg(argv, index, options, {
        presetFlag: null,
      })
      if (requirementIndex !== null) {
        index = requirementIndex
        continue
      }
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

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function matrixRootsFor(options) {
  const roots = [...options.matrixRoots]
  if (options.defaultRoots) {
    roots.push(
      path.join(options.ossRoot, ".artifacts", "drill-matrices"),
      path.join(options.cloudRoot, ".artifacts", "drill-matrices"),
    )
  }
  return [...new Set(roots.map((item) => path.resolve(item)))].sort()
}

function artifactRootsFor(options) {
  if (!options.includeDefaultArtifacts) return []
  return [
    path.join(options.ossRoot, ".artifacts"),
    path.join(options.cloudRoot, ".artifacts"),
  ].map((item) => path.resolve(item)).sort()
}

function failureRootsFor(options) {
  const roots = [...options.failureRoots]
  if (options.includeDefaultFailures) {
    roots.push(
      path.join(options.ossRoot, ".artifacts"),
      path.join(options.cloudRoot, ".artifacts"),
    )
  }
  return [...new Set(roots.map((item) => path.resolve(item)))].sort()
}

main().catch((error) => {
  console.error(`[drill-distributed-runtime-gate] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
