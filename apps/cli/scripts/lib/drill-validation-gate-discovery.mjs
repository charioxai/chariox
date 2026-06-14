import { readFile } from "node:fs/promises"

import {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import {
  DRILL_VALIDATION_GATE_SCHEMA,
  validateDrillValidationGateReport,
} from "./drill-validation-gate-report.mjs"

export async function readDrillValidationGateReport(reportPath) {
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  validateDrillValidationGateReport(report, reportPath)
  return report
}

export async function readDrillValidationGateAggregate(aggregatePath) {
  const aggregate = JSON.parse(await readFile(aggregatePath, "utf8"))
  validateDrillValidationGateAggregate(aggregate, aggregatePath)
  return aggregate
}

export async function findDrillValidationGateReportPaths(roots, { maxDepth = 8 } = {}) {
  return await findDrillJsonArtifactPaths(roots, {
    maxDepth,
    schema: DRILL_VALIDATION_GATE_SCHEMA,
  })
}

export async function findDrillValidationGateAggregatePaths(roots, { maxDepth = 8 } = {}) {
  return await findDrillJsonArtifactPaths(roots, {
    maxDepth,
    schema: DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  })
}
