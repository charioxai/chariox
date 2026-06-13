#!/usr/bin/env node
import {
  drillMatrixReportExitCode,
  formatDrillMatrixReportSummary,
  readDrillMatrixReport,
} from "./lib/drill-matrix-report.mjs"

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/drill-matrix-report-summary.mjs REPORT...",
    "",
    "Summarizes arroba.drill.matrix.v1 JSON reports and exits non-zero when any report failed.",
  ].join("\n"))
}

async function main() {
  const args = process.argv.slice(2)
  if (args.includes("--help") || args.includes("-h")) {
    printHelp()
    return
  }
  if (args.length === 0) {
    printHelp()
    process.exitCode = 1
    return
  }

  const reports = []
  for (const reportPath of args) {
    const report = await readDrillMatrixReport(reportPath)
    reports.push(report)
    console.log(formatDrillMatrixReportSummary(report, { source: reportPath }))
  }
  process.exitCode = drillMatrixReportExitCode(reports)
}

main().catch((error) => {
  console.error(`[drill-matrix-report-summary] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
