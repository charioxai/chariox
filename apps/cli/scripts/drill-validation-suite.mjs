#!/usr/bin/env node
import { spawn } from "node:child_process"
import {
  SHARED_DRILL_TEST_PATHS,
  drillValidationSuiteArtifactMetadata,
  drillValidationSuiteArgs,
  drillValidationSuiteCommand,
  drillValidationSuiteManifest,
  findMissingDrillValidationSuitePaths,
  findUnlistedDrillValidationSuitePaths,
} from "./lib/drill-validation-suite.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-suite.mjs [--list|--command|--check|--json|--run-json] [--output PATH]",
    "",
    "Runs the shared non-live drill validation suite.",
    "",
    "Options:",
    "  --check    Validate that every suite test path exists without running tests",
    "  --json     Print a machine-readable manifest of suite coverage",
    "  --run-json",
    "             Run the suite and print a machine-readable result report",
    "  --test-path PATH",
    "             Override suite test paths; may be repeated for focused validation",
    "  --output PATH",
    "             Write the --json manifest or --run-json report to PATH",
    "  --output-artifact-index PATH",
    "             Write an artifact index for --output",
    "  --list     Print test files included in the suite",
    "  --command  Print the node --test command without running it",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const testPaths = options.testPaths.length > 0 ? options.testPaths : SHARED_DRILL_TEST_PATHS
  if (options.help) {
    printHelp()
    return
  }
  if (options.list) {
    console.log(testPaths.join("\n"))
    return
  }
  if (options.command) {
    console.log(drillValidationSuiteCommand({ testPaths }))
    return
  }
  if (options.json) {
    const manifest = manifestForTestPaths(testPaths)
    if (options.outputPath) {
      await writeDrillJsonArtifactOutput({
        outputPath: options.outputPath,
        artifactIndexPath: options.outputArtifactIndexPath,
        value: manifest,
        metadata: drillValidationSuiteArtifactMetadata(manifest),
      })
    }
    console.log(JSON.stringify(manifest, null, 2))
    return
  }
  if (options.runJson) {
    const report = await runDrillValidationSuiteReport({ testPaths })
    if (options.outputPath) {
      await writeDrillJsonArtifactOutput({
        outputPath: options.outputPath,
        artifactIndexPath: options.outputArtifactIndexPath,
        value: report,
        metadata: drillValidationSuiteArtifactMetadata(report),
      })
    }
    console.log(JSON.stringify(report, null, 2))
    if (!report.ok) process.exitCode = report.exitCode ?? 1
    return
  }
  if (options.outputPath) throw new Error("--output requires --json or --run-json")
  if (options.outputArtifactIndexPath) throw new Error("--output-artifact-index requires --json or --run-json and --output")
  await assertDrillValidationSuiteComplete({ testPaths })
  if (options.check) {
    console.log(`validation suite paths ok (${testPaths.length} tests)`)
    return
  }

  const child = spawn(process.execPath, drillValidationSuiteArgs({ testPaths }), {
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

async function assertDrillValidationSuiteComplete({ testPaths = SHARED_DRILL_TEST_PATHS } = {}) {
  const missing = await findMissingDrillValidationSuitePaths({ testPaths })
  if (missing.length > 0) {
    throw new Error(`validation suite references missing test paths:\n${missing.map((item) => `- ${item}`).join("\n")}`)
  }
  if (testPaths === SHARED_DRILL_TEST_PATHS) {
    const unlisted = await findUnlistedDrillValidationSuitePaths()
    if (unlisted.length > 0) {
      throw new Error(`validation suite does not list discovered test paths:\n${unlisted.map((item) => `- ${item}`).join("\n")}`)
    }
  }
}

async function runDrillValidationSuiteReport({ testPaths = SHARED_DRILL_TEST_PATHS } = {}) {
  await assertDrillValidationSuiteComplete({ testPaths })
  const startedAt = new Date()
  const child = spawn(process.execPath, drillValidationSuiteArgs({ testPaths }), {
    cwd: process.cwd(),
    env: childTestProcessEnv(),
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.stdout.pipe(process.stderr)
  child.stderr.pipe(process.stderr)
  const result = await new Promise((resolve) => {
    child.on("exit", (code, signal) => resolve({ code, signal }))
    child.on("error", (error) => resolve({ code: 1, signal: null, error }))
  })
  const completedAt = new Date()
  const ok = result.code === 0 && !result.signal && !result.error
  if (result.error) {
    console.error(`[drill-validation-suite] ${result.error.stack ?? result.error.message}`)
  }
  if (result.signal) {
    console.error(`[drill-validation-suite] child exited with signal ${result.signal}`)
  }
  const manifest = manifestForTestPaths(testPaths)
  return {
    schema: "arroba.drill.validation_suite_run.v1",
    status: ok ? "passed" : "failed",
    ok,
    startedAt: startedAt.toISOString(),
    completedAt: completedAt.toISOString(),
    durationMs: completedAt.getTime() - startedAt.getTime(),
    exitCode: result.code,
    signal: result.signal,
    error: result.error ? String(result.error.message ?? result.error) : null,
    command: manifest.command,
    testCount: manifest.testCount,
    testPaths: manifest.testPaths,
    manifest,
  }
}

function manifestForTestPaths(testPaths) {
  if (testPaths === SHARED_DRILL_TEST_PATHS) return drillValidationSuiteManifest()
  return drillValidationSuiteManifest({
    testPaths,
    coverageAreas: [{
      id: "custom-suite",
      description: "Custom drill validation suite override.",
      testPaths,
    }],
    validationPresets: [],
  })
}

function childTestProcessEnv() {
  const env = { ...process.env }
  delete env.NODE_TEST_CONTEXT
  return env
}

function parseArgs(argv) {
  const options = {
    check: false,
    command: false,
    help: false,
    json: false,
    list: false,
    outputArtifactIndexPath: null,
    outputPath: null,
    runJson: false,
    testPaths: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--check") options.check = true
    else if (arg === "--json") options.json = true
    else if (arg === "--run-json") options.runJson = true
    else if (arg === "--list") options.list = true
    else if (arg === "--command") options.command = true
    else if (arg === "--test-path") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--test-path requires a value")
      options.testPaths.push(value)
      index += 1
    } else if (arg.startsWith("--test-path=")) {
      options.testPaths.push(arg.slice("--test-path=".length))
    }
    else if (arg === "--output") {
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
    }
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  const modes = [options.check, options.command, options.json, options.list, options.runJson].filter(Boolean).length
  if (modes > 1) throw new Error("choose only one of --list, --command, --check, --json, or --run-json")
  return options
}

main().catch((error) => {
  console.error(`[drill-validation-suite] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
