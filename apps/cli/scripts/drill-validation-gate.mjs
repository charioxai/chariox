#!/usr/bin/env node
import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import {
  drillValidationGateExitCode,
  formatDrillValidationGateSummary,
  runDrillValidationGate,
} from "./lib/drill-validation-gate.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-gate.mjs [options]",
    "",
    "Verifies collected validation platform artifacts for CI or staging gates.",
    "",
    "Options:",
    "  --platform-bundle DIR  Verify a drill platform bundle directory",
    "  --artifact-index PATH  Read and verify a specific artifact index; repeatable",
    "  --artifact-root ROOT   Discover artifact indexes below ROOT; repeatable",
    "  --matrix-report PATH    Read a specific matrix report; repeatable",
    "  --matrix-root ROOT     Discover matrix reports below ROOT; repeatable",
    "  --failure-manifest PATH",
    "                         Read a specific failure manifest or preserved root; repeatable",
    "  --failure-root ROOT    Discover failure manifests below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
    "  --require-complete     Fail when matrix reports include skipped or dry-run scenarios",
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
  const report = await runDrillValidationGate(options)
  if (options.outputPath) {
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: report,
      metadata: {
        drill: "validation-gate",
        status: report.status,
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
    matrixReports: [],
    matrixRoots: [],
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    platformBundleDir: null,
    requireComplete: false,
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--require-complete") options.requireComplete = true
    else if (arg === "--require-deployment-preset") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-deployment-preset requires a value")
      options.requiredDeploymentPresets.push(value)
      index += 1
    } else if (arg.startsWith("--require-deployment-preset=")) {
      options.requiredDeploymentPresets.push(arg.slice("--require-deployment-preset=".length))
    }
    else if (arg === "--require-provider") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-provider requires a value")
      options.requiredProviders.push(value)
      index += 1
    } else if (arg.startsWith("--require-provider=")) {
      options.requiredProviders.push(arg.slice("--require-provider=".length))
    }
    else if (arg === "--require-scenario") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-scenario requires a value")
      options.requiredScenarios.push(value)
      index += 1
    } else if (arg.startsWith("--require-scenario=")) {
      options.requiredScenarios.push(arg.slice("--require-scenario=".length))
    }
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
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-validation-gate] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
