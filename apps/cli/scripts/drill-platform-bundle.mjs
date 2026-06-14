#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"

import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"
import { drillValidationSuiteManifest } from "./lib/drill-validation-suite.mjs"

const BUNDLE_SCHEMA = "arroba.drill.platform_bundle.v1"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-platform-bundle.mjs --output-dir DIR",
    "",
    "Writes shared validation platform contract artifacts for CI or staging collection.",
    "",
    "Options:",
    "  --output-dir DIR  Directory where bundle JSON files are written",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
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

export async function writeDrillPlatformBundle(outputDir) {
  await mkdir(outputDir, { recursive: true })
  const files = [
    {
      path: "validation-suite.json",
      contents: drillValidationSuiteManifest(),
    },
    {
      path: "failure-taxonomy-scenario.json",
      contents: drillFailureTaxonomyManifest({ target: "scenario" }),
    },
    {
      path: "failure-taxonomy-drill.json",
      contents: drillFailureTaxonomyManifest({ target: "drill" }),
    },
  ]

  for (const file of files) {
    await writeFile(
      path.join(outputDir, file.path),
      `${JSON.stringify(file.contents, null, 2)}\n`,
      "utf8",
    )
  }

  const bundle = {
    schema: BUNDLE_SCHEMA,
    outputDir,
    artifacts: files.map((file) => ({
      path: file.path,
      schema: file.contents.schema,
    })),
  }
  await writeFile(path.join(outputDir, "index.json"), `${JSON.stringify(bundle, null, 2)}\n`, "utf8")
  return bundle
}

function parseArgs(argv) {
  const options = {
    help: false,
    outputDir: null,
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
