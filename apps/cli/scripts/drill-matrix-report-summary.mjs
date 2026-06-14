#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"
import {
  drillMatrixReportExitCode,
  drillMatrixReportCompletionExitCode,
  findDrillMatrixReportPaths,
  formatDrillMatrixReportSummary,
  readDrillMatrixReport,
  summarizeDrillMatrixReports,
} from "./lib/drill-matrix-report.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-matrix-report-summary.mjs [--json] [--output PATH] [--find ROOT] [--require-complete] REPORT...",
    "",
    "Summarizes arroba.drill.matrix.v1 JSON reports and exits non-zero when any report failed.",
    "",
    "Options:",
    "  --find ROOT    Discover valid matrix reports below ROOT; repeatable",
    "  --json         Print aggregate JSON instead of human-readable summaries",
    "  --output PATH  Write aggregate JSON to PATH",
    "  --require-complete",
    "                 Exit non-zero when selected reports contain skipped or dry-run scenarios",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const discovered = options.findRoots.length > 0
    ? await findDrillMatrixReportPaths(options.findRoots)
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
  const aggregate = summarizeDrillMatrixReports(reports)
  if (options.outputPath) {
    await mkdir(path.dirname(options.outputPath), { recursive: true })
    await writeFile(options.outputPath, `${JSON.stringify(aggregate, null, 2)}\n`, "utf8")
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
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
    } else if (arg === "--output") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--output requires a value")
      options.outputPath = value
      index += 1
    } else if (arg.startsWith("--output=")) {
      options.outputPath = arg.slice("--output=".length)
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown argument: ${arg}`)
    } else {
      options.reportPaths.push(arg)
    }
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-matrix-report-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
