#!/usr/bin/env node
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"
import { drillValidationSuiteManifest } from "./lib/drill-validation-suite.mjs"

const BUNDLE_SCHEMA = "arroba.drill.platform_bundle.v1"

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

export async function verifyDrillPlatformBundle(outputDir) {
  const indexPath = path.join(outputDir, "index.json")
  const bundle = JSON.parse(await readFile(indexPath, "utf8"))
  if (bundle.schema !== BUNDLE_SCHEMA) {
    throw new Error(`unsupported platform bundle schema ${JSON.stringify(bundle.schema)}`)
  }
  if (!Array.isArray(bundle.artifacts) || bundle.artifacts.length === 0) {
    throw new Error("platform bundle has no artifacts")
  }
  for (const artifact of bundle.artifacts) {
    if (!artifact || typeof artifact !== "object") throw new Error("platform bundle has invalid artifact entry")
    if (!relativeBundlePath(artifact.path)) throw new Error(`platform bundle has unsafe artifact path ${JSON.stringify(artifact.path)}`)
    if (typeof artifact.schema !== "string" || artifact.schema.length === 0) {
      throw new Error(`platform bundle artifact ${artifact.path} has invalid schema`)
    }
    const contents = JSON.parse(await readFile(path.join(outputDir, artifact.path), "utf8"))
    if (contents.schema !== artifact.schema) {
      throw new Error(`platform bundle artifact ${artifact.path} schema mismatch: expected ${artifact.schema}, got ${JSON.stringify(contents.schema)}`)
    }
  }
  return bundle
}

function relativeBundlePath(value) {
  return typeof value === "string"
    && value.length > 0
    && !path.isAbsolute(value)
    && !value.split(/[\\/]/).includes("..")
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
