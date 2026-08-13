#!/usr/bin/env node
import {
  findDrillFailureManifestPaths,
  formatDrillFailureManifestAggregateSummary,
  formatDrillFailureManifestSummary,
  readDrillFailureManifest,
  summarizeDrillFailureManifests,
} from "./lib/drill-failure-manifest.mjs"
import {
  parseDrillMaxDepth,
  parseDrillNonNegativeInteger,
} from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-failure-summary.mjs [--find] [--max-depth N] [--json] [--output PATH] FAILURE_ROOT_OR_MANIFEST...",
    "",
    "Summarizes chariox.drill.failure.v1 manifests from preserved drill artifact roots.",
    "Pass either a preserved drill root directory or an chariox-drill-failure.json path.",
    "",
    "Options:",
    "  --find          Recursively discover chariox-drill-failure.json files below each input directory",
    "  --max-depth N   Limit --find traversal depth; defaults to 8",
    "  --require-failure-max-age-ms MS",
    "                 Exit non-zero when failure manifests are older than this many milliseconds",
    "  --json          Print aggregate JSON instead of human-readable summaries",
    "  --output PATH   Write aggregate JSON to PATH",
    "  --output-artifact-index PATH",
    "                 Write an artifact index for --output",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.inputs.length === 0) {
    printHelp()
    process.exitCode = 1
    return
  }

  const inputs = options.find
    ? await findDrillFailureManifestPaths(options.inputs, { maxDepth: options.maxDepth })
    : options.inputs
  if (inputs.length === 0) {
    if (options.json) {
      console.log(JSON.stringify(emptyAggregate(), null, 2))
    } else {
      console.log("no drill failure manifests found")
    }
    return
  }
  const manifests = []
  for (const [index, input] of inputs.entries()) {
    const manifest = await readDrillFailureManifest(input)
    manifests.push(manifest)
    if (!options.json) {
      if (index > 0) console.log("")
      console.log(formatDrillFailureManifestSummary(manifest, { source: input }))
    }
  }
  const aggregate = summarizeDrillFailureManifests(manifests, {
    sources: inputs,
    requiredFailureMaxAgeMs: options.requiredFailureMaxAgeMs,
  })
  if (options.outputPath) {
    const runtimeSignals = Object.keys(aggregate.runtimeSignals).sort()
    const runtimeSignalOwners = Object.keys(aggregate.runtimeSignalOwners).sort()
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: aggregate,
      metadata: {
        drill: "failure-summary",
        total: aggregate.total,
        owners: Object.keys(aggregate.owners).join(","),
        classifications: Object.keys(aggregate.classifications).join(","),
        ...(runtimeSignals.length > 0
          ? {
            runtimeSignals: runtimeSignals.join(","),
            runtimeSignalOwners: runtimeSignalOwners.join(","),
          }
          : {}),
      },
    })
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
    if ((aggregate.staleFailureManifests ?? []).length > 0) process.exitCode = 1
    return
  }
  if (manifests.length > 1 || options.requiredFailureMaxAgeMs !== null) {
    console.log("")
    console.log(formatDrillFailureManifestAggregateSummary(aggregate))
  }
  if ((aggregate.staleFailureManifests ?? []).length > 0) process.exitCode = 1
}

function parseArgs(argv) {
  const options = {
    find: false,
    help: false,
    json: false,
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    requiredFailureMaxAgeMs: null,
    inputs: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (!arg) continue
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--find") options.find = true
    else if (arg === "--max-depth") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--max-depth requires a value")
      options.maxDepth = parseDrillMaxDepth(value)
      index += 1
    } else if (arg.startsWith("--max-depth=")) {
      options.maxDepth = parseDrillMaxDepth(arg.slice("--max-depth=".length))
    }
    else if (arg === "--require-failure-max-age-ms") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--require-failure-max-age-ms requires a value")
      options.requiredFailureMaxAgeMs = parseDrillNonNegativeInteger(value, "--require-failure-max-age-ms")
      index += 1
    } else if (arg.startsWith("--require-failure-max-age-ms=")) {
      options.requiredFailureMaxAgeMs = parseDrillNonNegativeInteger(
        arg.slice("--require-failure-max-age-ms=".length),
        "--require-failure-max-age-ms",
      )
    }
    else if (arg === "--json") options.json = true
    else if (arg === "--output") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output requires a value")
      options.outputPath = value
      index += 1
    } else if (arg.startsWith("--output=")) {
      options.outputPath = arg.slice("--output=".length)
    }
    else if (arg === "--output-artifact-index") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output-artifact-index requires a value")
      options.outputArtifactIndexPath = value
      index += 1
    } else if (arg.startsWith("--output-artifact-index=")) {
      options.outputArtifactIndexPath = arg.slice("--output-artifact-index=".length)
    }
    else if (arg.startsWith("--")) throw new Error(`unknown argument: ${arg}`)
    else options.inputs.push(arg)
  }
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  return options
}

function emptyAggregate() {
  return {
    schema: "chariox.drill.failure.aggregate.v1",
    total: 0,
    owners: {},
    classifications: {},
    runtimeSignals: {},
    runtimeSignalOwners: {},
    nextActions: [],
    failures: [],
  }
}

main().catch((error) => {
  console.error(`[drill-failure-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
