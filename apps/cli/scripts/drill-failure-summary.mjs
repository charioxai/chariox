#!/usr/bin/env node
import {
  findDrillFailureManifestPaths,
  formatDrillFailureManifestSummary,
  readDrillFailureManifest,
} from "./lib/drill-failure-manifest.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-failure-summary.mjs [--find] FAILURE_ROOT_OR_MANIFEST...",
    "",
    "Summarizes arroba.drill.failure.v1 manifests from preserved drill artifact roots.",
    "Pass either a preserved drill root directory or an arroba-drill-failure.json path.",
    "",
    "Options:",
    "  --find  Recursively discover arroba-drill-failure.json files below each input directory",
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
    console.log("no drill failure manifests found")
    return
  }
  for (const [index, input] of inputs.entries()) {
    const manifest = await readDrillFailureManifest(input)
    if (index > 0) console.log("")
    console.log(formatDrillFailureManifestSummary(manifest, { source: input }))
  }
}

function parseArgs(argv) {
  const options = {
    find: false,
    help: false,
    inputs: [],
  }
  for (const arg of argv) {
    if (!arg) continue
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--find") options.find = true
    else if (arg.startsWith("--")) throw new Error(`unknown argument: ${arg}`)
    else options.inputs.push(arg)
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-failure-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
