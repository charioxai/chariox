import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { matrixValidationGateCheck } from "./drill-validation-gate-matrix-check.mjs"

test("skips matrix validation when no matrix evidence or requirements are configured", async () => {
  const check = await matrixValidationGateCheck({
    matrixReports: [],
    matrixRoots: [],
  }, matrixOptions())

  assert.deepEqual(check, {
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
  })
})

test("fails when matrix coverage is required but no reports are found", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-matrix-"))
  try {
    const check = await matrixValidationGateCheck({
      matrixReports: [],
      matrixRoots: [rootDir],
    }, matrixOptions({
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredProviders: ["codex"],
    }))

    assert.equal(check.status, "failed")
    assert.equal(check.error, "no matrix reports found")
    assert.deepEqual(check.missingMatrices, ["workspace-live-sync-matrix"])
    assert.deepEqual(check.missingProviders, ["codex"])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("passes with matrix, classification, deployment, provider, and scenario coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-matrix-"))
  try {
    const reportPath = path.join(rootDir, "matrices", "workspace.json")
    await writeMatrixReport(reportPath, matrixReport({
      matrix: "workspace-live-sync-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "codex,opencode",
      },
      scenarios: [
        scenario("managed", "passed", { classification: "workspace-live-sync-conflict" }),
        scenario("permission", "passed", { classification: "kernel-authority" }),
      ],
    }))

    const check = await matrixValidationGateCheck({
      matrixReports: [],
      matrixRoots: [rootDir],
    }, matrixOptions({
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "workspace-live-sync-conflict"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
    }))

    assert.equal(check.status, "passed")
    assert.deepEqual(check.reportPaths, [reportPath])
    assert.deepEqual(check.missingMatrices, [])
    assert.deepEqual(check.missingMatrixClassifications, [])
    assert.deepEqual(check.missingDeploymentPresets, [])
    assert.deepEqual(check.missingProviders, [])
    assert.deepEqual(check.missingScenarios, [])
    assert.deepEqual(check.aggregate.matrixNames, { "workspace-live-sync-matrix": 1 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails incomplete dry-run reports only when complete execution is required", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-matrix-"))
  try {
    const reportPath = path.join(rootDir, "dry-run.json")
    await writeMatrixReport(reportPath, matrixReport({
      status: "dry-run",
      dryRun: true,
      durationMs: 0,
      completedAt: "2026-06-13T00:00:00.000Z",
      scenarios: [scenario("managed", "dry-run")],
    }))

    const permissive = await matrixValidationGateCheck({
      matrixReports: [reportPath],
      matrixRoots: [],
    }, matrixOptions())
    const strict = await matrixValidationGateCheck({
      matrixReports: [reportPath],
      matrixRoots: [],
    }, matrixOptions({ requireComplete: true }))

    assert.equal(permissive.status, "passed")
    assert.equal(strict.status, "failed")
    assert.equal(strict.aggregate.status, "dry-run")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("reports missing matrix coverage dimensions from otherwise valid evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-matrix-"))
  try {
    const reportPath = path.join(rootDir, "workspace.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: {
        deploymentPresets: "local",
        providers: "codex",
      },
      scenarios: [scenario("managed", "passed", { classification: "kernel-authority" })],
    }))

    const check = await matrixValidationGateCheck({
      matrixReports: [reportPath],
      matrixRoots: [],
    }, matrixOptions({
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredDeploymentPresets: ["hosted-cloud"],
      requiredProviders: ["claude"],
      requiredScenarios: ["tracked"],
    }))

    assert.equal(check.status, "failed")
    assert.deepEqual(check.missingMatrices, ["workspace-live-sync-matrix"])
    assert.deepEqual(check.missingMatrixClassifications, ["workspace-live-sync-conflict"])
    assert.deepEqual(check.missingDeploymentPresets, ["hosted-cloud"])
    assert.deepEqual(check.missingProviders, ["claude"])
    assert.deepEqual(check.missingScenarios, ["tracked"])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

function matrixOptions(overrides = {}) {
  return {
    maxDepth: 8,
    requireComplete: false,
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
    ...overrides,
  }
}

async function writeMatrixReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`, "utf8")
}

function matrixReport(overrides = {}) {
  const scenarios = overrides.scenarios ?? [scenario("local", "passed")]
  const status = overrides.status ?? matrixStatusForScenarios(scenarios)
  const dryRun = overrides.dryRun ?? status === "dry-run"
  const startedAt = overrides.startedAt ?? "2026-06-13T00:00:00.000Z"
  const durationMs = overrides.durationMs ?? 1000
  const completedAt = overrides.completedAt ?? new Date(Date.parse(startedAt) + durationMs).toISOString()
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status,
    dryRun,
    startedAt,
    completedAt,
    durationMs,
    metadata: {},
    scenarios,
    ...overrides,
  }
}

function matrixStatusForScenarios(scenarios) {
  if (scenarios.some((entry) => entry.status === "failed")) return "failed"
  if (scenarios.length > 0 && scenarios.every((entry) => entry.status === "dry-run")) return "dry-run"
  return "passed"
}

function scenario(id, status, overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status,
    expectedFailure: false,
    classification: null,
    durationMs: status === "skipped" || status === "dry-run" ? 0 : 10,
    reason: status === "skipped" ? "not run" : null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    ...overrides,
  }
}
