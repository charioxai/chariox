#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"

import { parseDrillMaxDepth } from "./lib/drill-cli-args.mjs"
import {
  drillValidationGateAggregateExitCode,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  readDrillValidationGateReport,
  summarizeDrillValidationGateReports,
} from "./lib/drill-validation-gate.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-validation-gate-summary.mjs [options]",
    "",
    "Aggregates persisted drill validation gate reports.",
    "",
    "Options:",
    "  --gate-report PATH     Read a specific validation gate report; repeatable",
    "  --gate-root ROOT       Discover validation gate reports below ROOT; repeatable",
    "  --max-depth N          Limit artifact discovery depth; defaults to 8",
    "  --json                 Print aggregate JSON",
    "  --output PATH          Write aggregate JSON to PATH",
  ].join("\n"))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const discovered = options.gateRoots.length > 0
    ? await findDrillValidationGateReportPaths(options.gateRoots, { maxDepth: options.maxDepth })
    : []
  const reportPaths = [...new Set([...options.gateReports, ...discovered])].sort()
  if (reportPaths.length === 0) {
    throw new Error("no validation gate reports found")
  }
  const reports = await Promise.all(reportPaths.map((reportPath) => readDrillValidationGateReport(reportPath)))
  const aggregate = summarizeDrillValidationGateReports(reports, { sources: reportPaths })
  if (options.outputPath) {
    await mkdir(path.dirname(options.outputPath), { recursive: true })
    await writeFile(options.outputPath, `${JSON.stringify(aggregate, null, 2)}\n`, "utf8")
  }
  if (options.json) {
    console.log(JSON.stringify(aggregate, null, 2))
  } else {
    console.log(formatDrillValidationGateAggregateSummary(aggregate))
  }
  process.exitCode = drillValidationGateAggregateExitCode(aggregate)
}

function parseArgs(argv) {
  const options = {
    gateReports: [],
    gateRoots: [],
    help: false,
    json: false,
    maxDepth: 8,
    outputPath: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--help" || arg === "-h") options.help = true
    else if (arg === "--json") options.json = true
    else if (arg === "--gate-report") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--gate-report requires a value")
      options.gateReports.push(value)
      index += 1
    } else if (arg.startsWith("--gate-report=")) {
      options.gateReports.push(arg.slice("--gate-report=".length))
    } else if (arg === "--gate-root") {
      const value = argv[index + 1]
      if (!value || value.startsWith("--")) throw new Error("--gate-root requires a value")
      options.gateRoots.push(value)
      index += 1
    } else if (arg.startsWith("--gate-root=")) {
      options.gateRoots.push(arg.slice("--gate-root=".length))
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
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown argument: ${arg}`)
    } else {
      throw new Error(`unexpected argument: ${arg}`)
    }
  }
  return options
}

main().catch((error) => {
  console.error(`[drill-validation-gate-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
