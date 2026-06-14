#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"

import { writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-failure-taxonomy.mjs [--target scenario|drill] [--output PATH]",
    "",
    "Prints the shared drill failure classification taxonomy as JSON.",
    "",
    "Options:",
    "  --target VALUE  Next-action target; scenario or drill. Defaults to scenario",
    "  --output PATH   Write the taxonomy JSON to PATH",
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

  const manifest = drillFailureTaxonomyManifest({ target: options.target })
  if (options.outputPath) {
    await mkdir(path.dirname(options.outputPath), { recursive: true })
    await writeFile(options.outputPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8")
    if (options.outputArtifactIndexPath) {
      await writeDrillArtifactIndex({
        rootDir: path.dirname(options.outputPath),
        artifacts: [path.basename(options.outputPath)],
        indexPath: options.outputArtifactIndexPath,
        metadata: {
          drill: "failure-taxonomy",
          target: manifest.target,
        },
      })
    }
  }
  console.log(JSON.stringify(manifest, null, 2))
}

function parseArgs(argv) {
  const options = {
    help: false,
    outputArtifactIndexPath: null,
    outputPath: null,
    target: "scenario",
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--target") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--target requires a value")
      options.target = parseTarget(value)
      index += 1
    } else if (arg.startsWith("--target=")) {
      options.target = parseTarget(arg.slice("--target=".length))
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

function parseTarget(value) {
  if (value === "scenario" || value === "drill") return value
  throw new Error(`unknown target: ${value}`)
}

main().catch((error) => {
  console.error(`[drill-failure-taxonomy] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
