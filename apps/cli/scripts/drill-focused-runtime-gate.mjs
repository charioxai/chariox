#!/usr/bin/env node
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  parseDrillMaxDepth,
  parseDrillNonNegativeInteger,
} from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  DRILL_FOCUSED_RUNTIME_GATE_SCHEMA,
  FOCUSED_RUNTIME_GATE_PRESETS,
  validateDrillFocusedRuntimeGateReport,
} from "./lib/drill-focused-runtime-gate-report.mjs"
import { diagnosticMetadataForValidationGateReport } from "./lib/drill-validation-gate-runtime-signal-metadata.mjs"
import { validationGateEvidenceSourceMetadata } from "./lib/drill-validation-gate-source-metadata.mjs"
import {
  formatDrillValidationGateSummary,
  runDrillValidationGate,
} from "./lib/drill-validation-gate.mjs"
import {
  parseValidationGateRequirementArg,
  validationGateRequirementOptionDefaults,
} from "./lib/drill-validation-gate-args.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultOssRoot = path.resolve(scriptDir, "..", "..", "..")
const defaultCloudRoot = path.resolve(defaultOssRoot, "..", "arroba-cloud")

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-focused-runtime-gate.mjs [options]",
    "",
    "Runs the focused runtime-authority and distributed-state-health validation gates.",
    "Use this as a fast inner-loop gate before the full distributed-runtime release gate.",
    "",
    "Options:",
    "  --oss-root DIR          OSS repo root; defaults to this script's repo root",
    "  --cloud-root DIR        Cloud repo root; defaults to ../arroba-cloud",
    "  --no-default-roots      Only use matrix roots passed explicitly with --matrix-root",
    "  --matrix-root ROOT      Discover matrix reports below ROOT; repeatable",
    "  --artifact-index PATH   Read and verify a specific artifact index; repeatable",
    "  --artifact-root ROOT    Discover artifact indexes below ROOT; repeatable",
    "  --include-default-artifacts",
    "                         Discover artifact indexes under each repo's .artifacts root",
    "  --failure-manifest PATH",
    "                         Read a specific failure manifest or preserved root; repeatable",
    "  --failure-root ROOT     Discover failure manifests below ROOT; repeatable",
    "  --include-default-failures",
    "                         Discover failure manifests under each repo's .artifacts root",
    "  --platform-bundle DIR   Use an existing drill platform bundle instead of generating one",
    "  --max-depth N           Limit artifact discovery depth; defaults to 8",
    "  --require-complete      Fail when matrix reports include skipped/dry-run scenarios or unresolved exit criteria",
    "  --require-artifact-max-age-ms MS",
    "  --require-failure-max-age-ms MS",
    "  --require-matrix-max-age-ms MS",
    "  --json                  Print focused gate report JSON",
    "  --output PATH           Write focused gate report JSON to PATH",
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

  const generatedBundleDir = options.platformBundleDir ? null : await mkdtemp(path.join(os.tmpdir(), "arroba-focused-runtime-platform-"))
  try {
    const platformBundleDir = options.platformBundleDir ?? generatedBundleDir
    if (!options.platformBundleDir) {
      await writeDrillPlatformBundle(platformBundleDir)
    }
    const reports = []
    for (const preset of FOCUSED_RUNTIME_GATE_PRESETS) {
      reports.push({
        preset,
        report: await runFocusedPresetGate(options, platformBundleDir, preset),
      })
    }
    const outputReport = {
      schema: DRILL_FOCUSED_RUNTIME_GATE_SCHEMA,
      status: reports.every(({ report }) => report.status === "passed") ? "passed" : "failed",
      presets: [...FOCUSED_RUNTIME_GATE_PRESETS],
      reports,
      nextActions: reports.flatMap(({ preset, report }) =>
        report.nextActions.map((action) => ({ ...action, preset })),
      ),
    }
    validateDrillFocusedRuntimeGateReport(outputReport)
    if (options.outputPath) {
      await writeDrillJsonArtifactOutput({
        outputPath: options.outputPath,
        artifactIndexPath: options.outputArtifactIndexPath,
        value: outputReport,
        metadata: focusedRuntimeGateArtifactMetadata(outputReport, options),
      })
    }
    if (options.json) {
      console.log(JSON.stringify(outputReport, null, 2))
    } else {
      console.log(formatFocusedRuntimeGateSummary(outputReport))
    }
    process.exitCode = outputReport.status === "passed" ? 0 : 1
  } finally {
    if (generatedBundleDir) {
      await rm(generatedBundleDir, { recursive: true, force: true }).catch(() => {})
    }
  }
}

async function runFocusedPresetGate(options, platformBundleDir, preset) {
  return await runDrillValidationGate({
    artifactIndexes: options.artifactIndexes,
    artifactRoots: artifactRootsFor(options),
    failureInputs: options.failureInputs,
    failureRoots: failureRootsFor(options),
    matrixRoots: matrixRootsFor(options),
    maxDepth: options.maxDepth,
    platformBundleDir,
    presets: [preset],
    requireComplete: options.requireComplete,
    requiredPlatformCoverageAreas: options.requiredPlatformCoverageAreas,
    requiredArtifactCoverageAreas: options.requiredArtifactCoverageAreas,
    requiredArtifactSchemas: options.requiredArtifactSchemas,
    requiredArtifactKinds: options.requiredArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds: options.requiredArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedEvidenceRepos: options.requiredArtifactGeneratedEvidenceRepos,
    requiredArtifactGeneratedMatrixArtifactIndexes: options.requiredArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: options.requiredArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: options.requiredArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: options.requiredArtifactGeneratedMatrixRepos,
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: options.requiredArtifactGeneratedValidationSuiteArtifactIndexes,
    requiredArtifactGeneratedValidationSuiteFailureRoots: options.requiredArtifactGeneratedValidationSuiteFailureRoots,
    requiredArtifactEvidenceRepos: options.requiredArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: options.requiredArtifactProviderAccountAliases,
    requiredArtifactValidationPresets: options.requiredArtifactValidationPresets,
    requiredArtifactRuntimeSignals: options.requiredArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: options.requiredArtifactRuntimeSignalOwners,
    requiredArtifactOwners: options.requiredArtifactOwners,
    requiredArtifactClassifications: options.requiredArtifactClassifications,
    requiredArtifactFailureClassifications: options.requiredArtifactFailureClassifications,
    requiredArtifactPlannedOwners: options.requiredArtifactPlannedOwners,
    requiredArtifactPlannedClassifications: options.requiredArtifactPlannedClassifications,
    requiredArtifactExitCriterionStatuses: options.requiredArtifactExitCriterionStatuses,
    requiredArtifactIncompleteExitCriterionStatuses: options.requiredArtifactIncompleteExitCriterionStatuses,
    requiredArtifactMaxAgeMs: options.requiredArtifactMaxAgeMs,
    requiredFailureMaxAgeMs: options.requiredFailureMaxAgeMs,
    requiredRuntimeSignals: options.requiredRuntimeSignals,
    requiredRuntimeSignalOwners: options.requiredRuntimeSignalOwners,
    requiredFailureClassifications: options.requiredFailureClassifications,
    requiredMatrices: options.requiredMatrices,
    requiredMatrixClassifications: options.requiredMatrixClassifications,
    requiredMatrixRuntimeSignals: options.requiredMatrixRuntimeSignals,
    requiredDeploymentPresets: options.requiredDeploymentPresets,
    requiredProviders: options.requiredProviders,
    requiredScenarios: options.requiredScenarios,
    requiredMatrixMaxAgeMs: options.requiredMatrixMaxAgeMs,
  })
}

function formatFocusedRuntimeGateSummary(report) {
  const lines = [
    "focused runtime gate:",
    `status=${report.status}`,
    `presets=${report.presets.join(",")}`,
  ]
  for (const { preset, report: presetReport } of report.reports) {
    lines.push(`preset=${preset} status=${presetReport.status}`)
    lines.push(...formatDrillValidationGateSummary(presetReport)
      .split("\n")
      .map((line) => `  ${line}`))
  }
  return lines.join("\n")
}

function parseArgs(argv) {
  const options = {
    artifactIndexes: [],
    artifactRoots: [],
    cloudRoot: defaultCloudRoot,
    defaultRoots: true,
    failureInputs: [],
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
    requiredArtifactMaxAgeMs: null,
    requiredFailureMaxAgeMs: null,
    requiredMatrixMaxAgeMs: null,
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
    } else if (arg === "--artifact-index") {
      options.artifactIndexes.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--artifact-index=")) {
      options.artifactIndexes.push(arg.slice("--artifact-index=".length))
    } else if (arg === "--artifact-root") {
      options.artifactRoots.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--artifact-root=")) {
      options.artifactRoots.push(arg.slice("--artifact-root=".length))
    } else if (arg === "--failure-manifest") {
      options.failureInputs.push(readValue(argv, index, arg))
      index += 1
    } else if (arg.startsWith("--failure-manifest=")) {
      options.failureInputs.push(arg.slice("--failure-manifest=".length))
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
    } else if (arg === "--require-artifact-max-age-ms") {
      options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-artifact-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-artifact-max-age-ms=")) {
      options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(arg.slice("--require-artifact-max-age-ms=".length), "--require-artifact-max-age-ms")
    } else if (arg === "--require-failure-max-age-ms") {
      options.requiredFailureMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-failure-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-failure-max-age-ms=")) {
      options.requiredFailureMaxAgeMs = parseDrillNonNegativeInteger(arg.slice("--require-failure-max-age-ms=".length), "--require-failure-max-age-ms")
    } else if (arg === "--require-matrix-max-age-ms") {
      options.requiredMatrixMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-matrix-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-matrix-max-age-ms=")) {
      options.requiredMatrixMaxAgeMs = parseDrillNonNegativeInteger(arg.slice("--require-matrix-max-age-ms=".length), "--require-matrix-max-age-ms")
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
  const roots = [...options.artifactRoots]
  if (options.includeDefaultArtifacts) {
    roots.push(
      path.join(options.ossRoot, ".artifacts"),
      path.join(options.cloudRoot, ".artifacts"),
    )
  }
  return [...new Set(roots.map((item) => path.resolve(item)))].sort()
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

function focusedRuntimeGateArtifactMetadata(report, options) {
  return {
    drill: "focused-runtime-gate",
    status: report.status,
    presets: report.presets.join(","),
    ...mergeMetadata(report.reports.map(({ report }) => validationGateEvidenceSourceMetadata(report, {
      ossRoot: options.ossRoot,
      cloudRoot: options.cloudRoot,
    }))),
    ...mergeMetadata(report.reports.map(({ report }) => diagnosticMetadataForValidationGateReport(report))),
  }
}

function mergeMetadata(metadataRecords) {
  const output = {}
  for (const metadata of metadataRecords) {
    for (const [key, value] of Object.entries(metadata)) {
      const values = String(value).split(",").filter(Boolean)
      output[key] = [...new Set([...(output[key]?.split(",") ?? []), ...values])].sort().join(",")
    }
  }
  return output
}

main().catch((error) => {
  console.error(`[drill-focused-runtime-gate] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
