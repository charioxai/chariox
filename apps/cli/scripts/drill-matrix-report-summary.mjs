#!/usr/bin/env node
import {
  drillMatrixReportExitCode,
  drillMatrixReportCompletionExitCode,
  findDrillMatrixReportPaths,
  formatDrillMatrixAggregateSummary,
  formatDrillMatrixReportSummary,
  readDrillMatrixReport,
  summarizeDrillMatrixReports,
} from "./lib/drill-matrix-report.mjs"
import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import { writeDrillJsonArtifactOutput } from "./lib/drill-artifacts.mjs"
import { drillRuntimeSignalOwnersFor } from "./lib/drill-runtime-signals.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-matrix-report-summary.mjs [--json] [--output PATH] [--find ROOT] [--max-depth N] [--require-complete] REPORT...",
    "",
    "Summarizes arroba.drill.matrix.v1 JSON reports and exits non-zero when any report failed.",
    "",
    "Options:",
    "  --find ROOT     Discover valid matrix reports below ROOT; repeatable",
    "  --max-depth N   Limit --find traversal depth; defaults to 8",
    "  --json          Print aggregate JSON instead of human-readable summaries",
    "  --output PATH   Write aggregate JSON to PATH",
    "  --output-artifact-index PATH",
    "                 Write an artifact index for --output",
    "  --require-complete",
    "                 Exit non-zero when selected reports contain skipped/dry-run scenarios or unresolved exit criteria",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const discovered = options.findRoots.length > 0
    ? await findDrillMatrixReportPaths(options.findRoots, { maxDepth: options.maxDepth })
    : []
  const reportPaths = [...new Set([...options.reportPaths, ...discovered])].sort()
  if (reportPaths.length === 0) {
    printHelp()
    process.exitCode = 1
    return
  }

  const reports = []
  for (const reportPath of reportPaths) {
    const report = await readDrillMatrixReport(reportPath)
    reports.push(report)
    if (!options.json) {
      console.log(formatDrillMatrixReportSummary(report, { source: reportPath }))
    }
  }
  const aggregate = summarizeDrillMatrixReports(reports, { sources: reportPaths })
  if (options.outputPath) {
    const runtimeSignals = Object.keys(aggregate.runtimeSignals).sort()
    await writeDrillJsonArtifactOutput({
      outputPath: options.outputPath,
      artifactIndexPath: options.outputArtifactIndexPath,
      value: aggregate,
      metadata: {
        drill: "matrix-report-summary",
        status: aggregate.status,
        owners: Object.keys(aggregate.owners).join(","),
        classifications: Object.keys(aggregate.classifications).join(","),
        runtimeSignals: runtimeSignals.join(","),
        runtimeSignalOwners: drillRuntimeSignalOwnersFor(runtimeSignals).join(","),
      },
    })
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
  } else if (reports.length > 1) {
    console.log(formatDrillMatrixAggregateSummary(aggregate))
  }
  process.exitCode = options.requireComplete
    ? drillMatrixReportCompletionExitCode(reports)
    : drillMatrixReportExitCode(reports)
}

function parseArgs(argv) {
  const options = {
    help: false,
    json: false,
    requireComplete: false,
    findRoots: [],
    maxDepth: 8,
    outputArtifactIndexPath: null,
    outputPath: null,
    reportPaths: [],
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--require-complete") options.requireComplete = true
    else if (arg === "--find") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--find requires a value")
      options.findRoots.push(value)
      index += 1
    } else if (arg.startsWith("--find=")) {
      options.findRoots.push(arg.slice("--find=".length))
    } else if (arg === "--max-depth") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--max-depth requires a value")
      options.maxDepth = parseDrillMaxDepth(value)
      index += 1
    } else if (arg.startsWith("--max-depth=")) {
      options.maxDepth = parseDrillMaxDepth(arg.slice("--max-depth=".length))
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
      options.reportPaths.push(arg)
    }
  }
  if (options.outputArtifactIndexPath && !options.outputPath) {
    throw new Error("--output-artifact-index requires --output")
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-matrix-report-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
