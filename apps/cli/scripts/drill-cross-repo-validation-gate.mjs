#!/usr/bin/env node
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

import {
  parseDrillMaxDepth,
  parseDrillNonNegativeInteger,
} from "./lib/drill-cli-args.mjs"
import {
  drillGeneratedMatrixNamesManifest,
} from "./lib/drill-generated-matrix-names.mjs"
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
import { validationGateEvidenceSourceMetadata } from "./lib/drill-validation-gate-source-metadata.mjs"

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
    "  --require-generated-matrix-registry-parity",
    "                         Fail when OSS and Cloud generated matrix registries drift",
    "  --platform-bundle DIR  Verify a drill platform bundle directory",
    "  --matrix-root ROOT     Discover matrix reports below ROOT; repeatable",
    "  --artifact-index PATH  Read and verify a specific artifact index; repeatable",
    "  --artifact-root ROOT   Discover artifact indexes below ROOT; repeatable",
    "  --failure-manifest PATH",
    "                         Read a specific failure manifest or preserved root; repeatable",
    "  --failure-root ROOT    Discover failure manifests below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
  "  --require-complete     Fail when matrix reports include skipped/dry-run scenarios or unresolved exit criteria",
  "  --preset NAME[,NAME]   Apply named requirement preset; repeatable",
  `                         Known: ${Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort().join(", ")}`,
  "  --require-platform-coverage-area ID[,ID]",
  "  --require-artifact-coverage-area ID[,ID]",
  "  --require-artifact-schema SCHEMA[,SCHEMA]",
  "  --require-artifact-kind KIND[,KIND]",
  "  --require-artifact-generated-evidence-kind KIND[,KIND]",
  "  --require-artifact-generated-matrix-artifact-index PATH[,PATH]",
  "  --require-artifact-generated-matrix-limitation KIND[,KIND]",
  "  --require-artifact-evidence-repo REPO[,REPO]",
  "  --require-artifact-provider-account-alias P=A[,P=A]",
  "  --require-artifact-runtime-signal ID[,ID]",
  "  --require-artifact-runtime-signal-owner OWNER[,OWNER]",
  "  --require-artifact-owner OWNER[,OWNER]",
  "  --require-artifact-classification KIND[,KIND]",
  "  --require-artifact-exit-criterion-status STATUS[,STATUS]",
  "  --require-artifact-incomplete-exit-criterion-status STATUS[,STATUS]",
  "  --require-artifact-max-age-ms MS",
    "  --require-failure-max-age-ms MS",
    "  --require-runtime-signal ID[,ID]",
    "  --require-runtime-signal-owner OWNER[,OWNER]",
    "  --require-failure-classification KIND[,KIND]",
    "  --require-matrix NAME[,NAME]",
    "  --require-matrix-classification KIND[,KIND]",
    "  --require-matrix-runtime-signal ID[,ID]",
    "  --require-deployment-preset NAME[,NAME]",
    "  --require-provider NAME[,NAME]",
    "  --require-scenario ID[,ID]",
    "  --require-matrix-max-age-ms MS",
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
  if (options.requireGeneratedMatrixRegistryParity) {
    await verifyGeneratedMatrixRegistryParity(options)
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
        ...validationGateEvidenceSourceMetadata(report, {
          ossRoot: options.ossRoot,
          cloudRoot: options.cloudRoot,
        }),
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
    cloudRoot: defaultCloudRoot,
    defaultRoots: true,
    failureInputs: [],
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
    requireGeneratedMatrixRegistryParity: false,
    requiredArtifactMaxAgeMs: null,
    requiredFailureMaxAgeMs: null,
    requiredMatrixMaxAgeMs: null,
    ...validationGateRequirementOptionDefaults({ presetKey: "presets" }),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--no-default-roots") options.defaultRoots = false
    else if (arg === "--include-default-artifacts") options.includeDefaultArtifacts = true
    else if (arg === "--include-default-failures") options.includeDefaultFailures = true
    else if (arg === "--require-generated-matrix-registry-parity") options.requireGeneratedMatrixRegistryParity = true
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
      } else if (arg === "--max-depth") {
        options.maxDepth = parseDrillMaxDepth(readValue(argv, index, arg))
        index += 1
      } else if (arg.startsWith("--max-depth=")) {
        options.maxDepth = parseDrillMaxDepth(arg.slice("--max-depth=".length))
      } else if (arg === "--require-artifact-max-age-ms") {
        options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-artifact-max-age-ms")
        index += 1
      } else if (arg.startsWith("--require-artifact-max-age-ms=")) {
        options.requiredArtifactMaxAgeMs = parseDrillNonNegativeInteger(
          arg.slice("--require-artifact-max-age-ms=".length),
          "--require-artifact-max-age-ms",
        )
      } else if (arg === "--require-failure-max-age-ms") {
        options.requiredFailureMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-failure-max-age-ms")
        index += 1
      } else if (arg.startsWith("--require-failure-max-age-ms=")) {
        options.requiredFailureMaxAgeMs = parseDrillNonNegativeInteger(
          arg.slice("--require-failure-max-age-ms=".length),
          "--require-failure-max-age-ms",
        )
      } else if (arg === "--require-matrix-max-age-ms") {
        options.requiredMatrixMaxAgeMs = parseDrillNonNegativeInteger(readValue(argv, index, arg), "--require-matrix-max-age-ms")
        index += 1
      } else if (arg.startsWith("--require-matrix-max-age-ms=")) {
        options.requiredMatrixMaxAgeMs = parseDrillNonNegativeInteger(
          arg.slice("--require-matrix-max-age-ms=".length),
          "--require-matrix-max-age-ms",
        )
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
  if (options.requiredGeneratedEvidenceKinds.length > 0) {
    throw new Error("--require-generated-evidence-kind is supported by drill-validation-gate-summary.mjs after validation gate reports are written")
  }
  if (options.requiredGeneratedMatrixArtifactIndexes.length > 0) {
    throw new Error("--require-generated-matrix-artifact-index is supported by drill-validation-gate-summary.mjs after validation gate reports are written")
  }
  if (options.requiredGeneratedMatrixLimitations.length > 0) {
    throw new Error("--require-generated-matrix-limitation is supported by drill-validation-gate-summary.mjs after validation gate reports are written")
  }
  if (options.requiredGeneratedValidationSuiteArtifactIndexes.length > 0) {
    throw new Error("--require-generated-validation-suite-artifact-index is supported by drill-validation-gate-summary.mjs after validation gate reports are written")
  }
  if (options.requiredGeneratedValidationSuiteFailureRoots.length > 0) {
    throw new Error("--require-generated-validation-suite-failure-root is supported by drill-validation-gate-summary.mjs after validation gate reports are written")
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
    artifactIndexes: uniqueSortedResolvedPaths(options.artifactIndexes),
    artifactRoots: uniqueSortedResolvedPaths(artifactRoots),
    failureInputs: uniqueSortedResolvedPaths(options.failureInputs),
    failureRoots: uniqueSortedResolvedPaths(failureRoots),
    matrixRoots: uniqueSortedResolvedPaths(matrixRoots),
    maxDepth: options.maxDepth,
    platformBundleDir: options.platformBundleDir,
    presets: options.presets,
    requireComplete: options.requireComplete,
    requiredPlatformCoverageAreas: options.requiredPlatformCoverageAreas,
    requiredArtifactCoverageAreas: options.requiredArtifactCoverageAreas,
    requiredArtifactSchemas: options.requiredArtifactSchemas,
    requiredArtifactKinds: options.requiredArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds: options.requiredArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedMatrixArtifactIndexes: options.requiredArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: options.requiredArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: options.requiredArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: options.requiredArtifactGeneratedMatrixRepos,
    requiredArtifactEvidenceRepos: options.requiredArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: options.requiredArtifactProviderAccountAliases,
    requiredArtifactRuntimeSignals: options.requiredArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: options.requiredArtifactRuntimeSignalOwners,
    requiredArtifactOwners: options.requiredArtifactOwners,
    requiredArtifactClassifications: options.requiredArtifactClassifications,
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
  }
}

function uniqueSortedResolvedPaths(paths) {
  return [...new Set(paths.map((item) => path.resolve(item)))].sort()
}

async function verifyGeneratedMatrixRegistryParity(options) {
  const cloudRegistryPath = path.join(options.cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  let cloudModule
  try {
    cloudModule = await import(pathToFileURL(cloudRegistryPath).href)
  } catch (error) {
    throw new Error(`generated matrix registry parity requires Cloud registry at ${cloudRegistryPath}: ${error.message}`)
  }
  if (typeof cloudModule.cloudDrillGeneratedMatrixNamesManifest !== "function") {
    throw new Error(`generated matrix registry parity requires cloudDrillGeneratedMatrixNamesManifest in ${cloudRegistryPath}`)
  }
  const ossRegistry = generatedMatrixRepoMap(drillGeneratedMatrixNamesManifest(), "OSS generated matrix registry")
  const cloudRegistry = generatedMatrixRepoMap(
    cloudModule.cloudDrillGeneratedMatrixNamesManifest(),
    "Cloud generated matrix registry",
  )
  if (JSON.stringify(ossRegistry) !== JSON.stringify(cloudRegistry)) {
    throw new Error(
      "generated matrix registry parity failed: "
        + `OSS=${JSON.stringify(ossRegistry)} Cloud=${JSON.stringify(cloudRegistry)}`,
    )
  }
}

function generatedMatrixRepoMap(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (!Array.isArray(manifest.matrices)) {
    throw new Error(`${source} has invalid matrices`)
  }
  return Object.fromEntries(manifest.matrices
    .map((matrix, index) => {
      if (!matrix || typeof matrix !== "object" || Array.isArray(matrix)) {
        throw new Error(`${source}.matrices[${index}] is not an object`)
      }
      if (typeof matrix.name !== "string" || matrix.name.length === 0) {
        throw new Error(`${source}.matrices[${index}] has invalid name`)
      }
      if (typeof matrix.repo !== "string" || matrix.repo.length === 0) {
        throw new Error(`${source}.matrices[${index}] has invalid repo`)
      }
      return [matrix.name, matrix.repo]
    })
    .sort(([left], [right]) => left.localeCompare(right)))
}

main().catch((error) => {
  console.error(`[drill-cross-repo-validation-gate] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
