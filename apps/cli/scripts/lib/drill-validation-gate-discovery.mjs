import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"

import {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"
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
  const discovered = new Set()
  for (const root of roots) {
    await collectDrillValidationGateReportPaths(discovered, root, { depth: 0, maxDepth })
  }
  return [...discovered].sort()
}

export async function findDrillValidationGateAggregatePaths(roots, { maxDepth = 8 } = {}) {
  const discovered = new Set()
  for (const root of roots) {
    await collectDrillValidationGateAggregatePaths(discovered, root, { depth: 0, maxDepth })
  }
  return [...discovered].sort()
}

async function collectDrillValidationGateReportPaths(discovered, entryPath, { depth, maxDepth }) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isFile()) {
    await maybeCollectDrillValidationGatePath(discovered, entryPath, DRILL_VALIDATION_GATE_SCHEMA)
    return
  }
  if (!entry.isDirectory() || depth > maxDepth) return
  let dir = null
  try {
    dir = await opendir(entryPath)
    for await (const child of dir) {
      const childPath = path.join(entryPath, child.name)
      if (child.isFile()) {
        await maybeCollectDrillValidationGatePath(discovered, childPath, DRILL_VALIDATION_GATE_SCHEMA)
        continue
      }
      if (!child.isDirectory() || shouldPruneValidationGateDirectory(child.name)) continue
      await collectDrillValidationGateReportPaths(discovered, childPath, { depth: depth + 1, maxDepth })
    }
  } catch {
    // Ignore unreadable directories in broad artifact roots.
  }
}

async function collectDrillValidationGateAggregatePaths(discovered, entryPath, { depth, maxDepth }) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isFile()) {
    await maybeCollectDrillValidationGatePath(discovered, entryPath, DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA)
    return
  }
  if (!entry.isDirectory() || depth > maxDepth) return
  let dir = null
  try {
    dir = await opendir(entryPath)
    for await (const child of dir) {
      const childPath = path.join(entryPath, child.name)
      if (child.isFile()) {
        await maybeCollectDrillValidationGatePath(discovered, childPath, DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA)
        continue
      }
      if (!child.isDirectory() || shouldPruneValidationGateDirectory(child.name)) continue
      await collectDrillValidationGateAggregatePaths(discovered, childPath, { depth: depth + 1, maxDepth })
    }
  } catch {
    // Ignore unreadable directories in broad artifact roots.
  }
}

async function maybeCollectDrillValidationGatePath(discovered, entryPath, schema) {
  if (!entryPath.endsWith(".json")) return
  try {
    const parsed = JSON.parse(await readFile(entryPath, "utf8"))
    if (parsed?.schema === schema) discovered.add(entryPath)
  } catch {
    // Ignore unrelated JSON files in broad artifact roots.
  }
}

function shouldPruneValidationGateDirectory(name) {
  return name === ".git"
    || name === "node_modules"
    || name === ".pnpm-store"
    || name === "debug"
    || name === "release"
}
