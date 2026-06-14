#!/usr/bin/env node
import { spawn } from "node:child_process"
import {
  SHARED_DRILL_TEST_PATHS,
  drillValidationSuiteArgs,
  drillValidationSuiteCommand,
} from "./lib/drill-validation-suite.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-suite.mjs [--list|--command]",
    "",
    "Runs the shared non-live drill validation suite.",
    "",
    "Options:",
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
    command: false,
    help: false,
    list: false,
  }
  for (const arg of argv) {
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--list") options.list = true
    else if (arg === "--command") options.command = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-validation-suite] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
