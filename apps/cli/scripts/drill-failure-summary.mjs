#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import {
  findDrillFailureManifestPaths,
  formatDrillFailureManifestAggregateSummary,
  formatDrillFailureManifestSummary,
  readDrillFailureManifest,
  summarizeDrillFailureManifests,
} from "./lib/drill-failure-manifest.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-failure-summary.mjs [--find] [--json] [--output PATH] FAILURE_ROOT_OR_MANIFEST...",
    "",
    "Summarizes arroba.drill.failure.v1 manifests from preserved drill artifact roots.",
    "Pass either a preserved drill root directory or an arroba-drill-failure.json path.",
    "",
    "Options:",
    "  --find         Recursively discover arroba-drill-failure.json files below each input directory",
    "  --json         Print aggregate JSON instead of human-readable summaries",
    "  --output PATH  Write aggregate JSON to PATH",
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

  const inputs = options.find ? await findDrillFailureManifestPaths(options.inputs) : options.inputs
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
  const aggregate = summarizeDrillFailureManifests(manifests, { sources: inputs })
  if (options.outputPath) {
    await mkdir(path.dirname(options.outputPath), { recursive: true })
    await writeFile(options.outputPath, `${JSON.stringify(aggregate, null, 2)}\n`, "utf8")
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
    return
  }
  if (manifests.length > 1) {
    console.log("")
    console.log(formatDrillFailureManifestAggregateSummary(aggregate))
  }
}

function parseArgs(argv) {
  const options = {
    find: false,
    help: false,
    json: false,
    outputPath: null,
    inputs: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (!arg) continue
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--find") options.find = true
    else if (arg === "--json") options.json = true
    else if (arg === "--output") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output requires a value")
      options.outputPath = value
      index += 1
    } else if (arg.startsWith("--output=")) {
      options.outputPath = arg.slice("--output=".length)
    }
    else if (arg.startsWith("--")) throw new Error(`unknown argument: ${arg}`)
    else options.inputs.push(arg)
  }
  return options
}

function emptyAggregate() {
  return {
    schema: "arroba.drill.failure.aggregate.v1",
    total: 0,
    owners: {},
    classifications: {},
    nextActions: [],
    failures: [],
  }
}

main().catch((error) => {
  console.error(`[drill-failure-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
