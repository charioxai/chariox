#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import {
  SHARED_DRILL_TEST_PATHS,
  drillValidationSuiteArgs,
  drillValidationSuiteCommand,
  drillValidationSuiteManifest,
  findMissingDrillValidationSuitePaths,
} from "./lib/drill-validation-suite.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-suite.mjs [--list|--command|--check|--json] [--output PATH]",
    "",
    "Runs the shared non-live drill validation suite.",
    "",
    "Options:",
    "  --check    Validate that every suite test path exists without running tests",
    "  --json     Print a machine-readable manifest of suite coverage",
    "  --output PATH",
    "             Write the --json manifest to PATH",
    "  --list     Print test files included in the suite",
    "  --command  Print the node --test command without running it",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.list) {
    console.log(SHARED_DRILL_TEST_PATHS.join("\n"))
    return
  }
  if (options.command) {
    console.log(drillValidationSuiteCommand())
    return
  }
  if (options.json) {
    const manifest = drillValidationSuiteManifest()
    if (options.outputPath) {
      await mkdir(path.dirname(options.outputPath), { recursive: true })
      await writeFile(options.outputPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8")
    }
    console.log(JSON.stringify(manifest, null, 2))
    return
  }
  if (options.outputPath) throw new Error("--output requires --json")
  const missing = await findMissingDrillValidationSuitePaths()
  if (missing.length > 0) {
    throw new Error(`validation suite references missing test paths:\n${missing.map((item) => `- ${item}`).join("\n")}`)
  }
  if (options.check) {
    console.log(`validation suite paths ok (${SHARED_DRILL_TEST_PATHS.length} tests)`)
    return
  }

  const child = spawn(process.execPath, drillValidationSuiteArgs(), {
    cwd: process.cwd(),
    stdio: "inherit",
  })
  const result = await new Promise((resolve) => {
    child.on("exit", (code, signal) => resolve({ code, signal }))
    child.on("error", (error) => resolve({ code: 1, signal: null, error }))
  })
  if (result.error) {
    console.error(`[drill-validation-suite] ${result.error.stack ?? result.error.message}`)
  }
  if (result.signal) {
    console.error(`[drill-validation-suite] child exited with signal ${result.signal}`)
  }
  process.exitCode = result.code ?? 1
}

function parseArgs(argv) {
  const options = {
    check: false,
    command: false,
    help: false,
    json: false,
    list: false,
    outputPath: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--check") options.check = true
    else if (arg === "--json") options.json = true
    else if (arg === "--list") options.list = true
    else if (arg === "--command") options.command = true
    else if (arg === "--output") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output requires a value")
      options.outputPath = value
      index += 1
    } else if (arg.startsWith("--output=")) {
      options.outputPath = arg.slice("--output=".length)
    }
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-validation-suite] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
