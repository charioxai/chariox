#!/usr/bin/env node
import {
  verifyDrillPlatformBundle,
  writeDrillPlatformBundle,
} from "./lib/drill-platform-bundle.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-platform-bundle.mjs --output-dir DIR",
    "       node apps/cli/scripts/drill-platform-bundle.mjs --verify-dir DIR",
    "",
    "Writes shared validation platform contract artifacts for CI or staging collection.",
    "",
    "Options:",
    "  --output-dir DIR  Directory where bundle JSON files are written",
    "  --verify-dir DIR  Validate a previously written bundle directory",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.outputDir && options.verifyDir) throw new Error("--output-dir and --verify-dir are mutually exclusive")
  if (options.verifyDir) {
    const bundle = await verifyDrillPlatformBundle(options.verifyDir)
    console.log(JSON.stringify(bundle, null, 2))
    return
  }
  if (!options.outputDir) {
    printHelp()
    process.exitCode = 1
    return
  }

  const bundle = await writeDrillPlatformBundle(options.outputDir)
  console.log(JSON.stringify(bundle, null, 2))
}

function parseArgs(argv) {
  const options = {
    help: false,
    outputDir: null,
    verifyDir: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--output-dir") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output-dir requires a value")
      options.outputDir = value
      index += 1
    } else if (arg.startsWith("--output-dir=")) {
      options.outputDir = arg.slice("--output-dir=".length)
    } else if (arg === "--verify-dir") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--verify-dir requires a value")
      options.verifyDir = value
      index += 1
    } else if (arg.startsWith("--verify-dir=")) {
      options.verifyDir = arg.slice("--verify-dir=".length)
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown argument: ${arg}`)
    } else {
      throw new Error(`unexpected argument: ${arg}`)
    }
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-platform-bundle] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
