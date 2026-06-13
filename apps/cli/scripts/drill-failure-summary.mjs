#!/usr/bin/env node
import {
  formatDrillFailureManifestSummary,
  readDrillFailureManifest,
} from "./lib/drill-failure-manifest.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-failure-summary.mjs FAILURE_ROOT_OR_MANIFEST...",
    "",
    "Summarizes arroba.drill.failure.v1 manifests from preserved drill artifact roots.",
    "Pass either a preserved drill root directory or an arroba-drill-failure.json path.",
  ].join("\n"))
}

async function main() {
  const inputs = process.argv.slice(2).filter((arg) => arg !== "")
  if (inputs.includes("--help") || inputs.includes("-h")) {
    printHelp()
    return
  }
  if (inputs.length === 0) {
    printHelp()
    process.exitCode = 1
    return
  }

  for (const [index, input] of inputs.entries()) {
    const manifest = await readDrillFailureManifest(input)
    if (index > 0) console.log("")
    console.log(formatDrillFailureManifestSummary(manifest, { source: input }))
  }
}

main().catch((error) => {
  console.error(`[drill-failure-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
