import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { summarizeValidationGateReportAggregate } from "./drill-validation-gate-aggregate.mjs"
import {
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
} from "./drill-validation-gate-discovery.mjs"
import {
  DRILL_VALIDATION_GATE_SCHEMA,
  validateDrillValidationGateReport,
} from "./drill-validation-gate-report.mjs"

test("reads validation gate reports and rejects invalid report files", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-discovery-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    const invalidPath = path.join(rootDir, "invalid.json")
    const value = report()
    await writeJson(reportPath, value)
    await writeJson(invalidPath, { schema: "wrong" })

    assert.deepEqual(await readDrillValidationGateReport(reportPath), value)
    await assert.rejects(
      () => readDrillValidationGateReport(invalidPath),
      /unsupported schema/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("reads validation gate aggregates and rejects invalid aggregate files", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-discovery-"))
  try {
    const aggregatePath = path.join(rootDir, "aggregate.json")
    const invalidPath = path.join(rootDir, "invalid-aggregate.json")
    const aggregate = summarizeValidationGateReportAggregate([report()], {
      sources: ["gate.json"],
      validateReport: validateDrillValidationGateReport,
    })
    await writeJson(aggregatePath, aggregate)
    await writeJson(invalidPath, { schema: "wrong" })

    assert.deepEqual(await readDrillValidationGateAggregate(aggregatePath), JSON.parse(JSON.stringify(aggregate)))
    await assert.rejects(
      () => readDrillValidationGateAggregate(invalidPath),
      /unsupported schema/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("discovers validation gate reports below broad roots", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-discovery-"))
  try {
    const first = path.join(rootDir, "reports", "one.json")
    const second = path.join(rootDir, ".artifacts", "reports", "two.json")
    const pruned = path.join(rootDir, "node_modules", "package", "ignored.json")
    const unrelated = path.join(rootDir, "reports", "other.json")
    const malformed = path.join(rootDir, "reports", "malformed.json")
    const nonJson = path.join(rootDir, "reports", "gate.txt")
    await writeJson(first, report())
    await writeJson(second, report())
    await writeJson(pruned, report())
    await writeJson(unrelated, { schema: "other" })
    await writeFileWithDir(malformed, "{not json}\n")
    await writeFileWithDir(nonJson, JSON.stringify(report()))

    assert.deepEqual(await findDrillValidationGateReportPaths([rootDir]), [second, first].sort())
    assert.deepEqual(await findDrillValidationGateReportPaths([first]), [first])
    assert.deepEqual(await findDrillValidationGateReportPaths([path.join(rootDir, "missing")]), [])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("discovers validation gate aggregates and respects max depth", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-discovery-"))
  try {
    const aggregate = summarizeValidationGateReportAggregate([report()], {
      validateReport: validateDrillValidationGateReport,
    })
    const shallow = path.join(rootDir, "aggregate.json")
    const deep = path.join(rootDir, "one", "two", "aggregate.json")
    const reportPath = path.join(rootDir, "gate.json")
    await writeJson(shallow, aggregate)
    await writeJson(deep, aggregate)
    await writeJson(reportPath, report())

    assert.deepEqual(await findDrillValidationGateAggregatePaths([rootDir]), [shallow, deep].sort())
    assert.deepEqual(await findDrillValidationGateAggregatePaths([rootDir], { maxDepth: 1 }), [shallow])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

function report(overrides = {}) {
  const checks = {
    configuration: { status: "passed" },
    platformBundle: {
      status: "skipped",
      dir: null,
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    },
    artifacts: { status: "skipped", roots: [], inputs: [], indexPaths: [] },
    matrices: {
      status: "skipped",
      roots: [],
      inputs: [],
      reportPaths: [],
      requireComplete: false,
      requiredMatrices: [],
      missingMatrices: [],
      requiredMatrixClassifications: [],
      missingMatrixClassifications: [],
      requiredDeploymentPresets: [],
      missingDeploymentPresets: [],
      requiredProviders: [],
      missingProviders: [],
      requiredScenarios: [],
      missingScenarios: [],
    },
    failures: { status: "skipped", roots: [], inputs: [], manifestPaths: [] },
    ...(overrides.checks ?? {}),
  }
  return {
    schema: DRILL_VALIDATION_GATE_SCHEMA,
    status: Object.values(checks).some((check) => check.status === "failed") ? "failed" : "passed",
    presets: [],
    checks,
    nextActions: [],
    ...overrides,
    checks,
  }
}

async function writeJson(file, value) {
  await writeFileWithDir(file, `${JSON.stringify(value, null, 2)}\n`)
}

async function writeFileWithDir(file, contents) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, "utf8")
}
