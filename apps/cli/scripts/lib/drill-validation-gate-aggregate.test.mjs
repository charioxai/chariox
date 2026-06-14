import assert from "node:assert/strict"
import test from "node:test"

import {
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"

test("summarizes validation gate reports with aggregate requirements", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    sources: ["workspace-live-sync.json"],
    normalizedRequiredPresets: ["workspace-live-sync"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["runtime-fixtures"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "passed")
  assert.equal(drillValidationGateAggregateExitCode(aggregate), 0)
  assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
  assert.deepEqual(aggregate.coverage.presets, { "workspace-live-sync": 1 })
  assert.deepEqual(aggregate.missingPresets, [])
  assert.deepEqual(aggregate.missingProviders, [])
  assert.deepEqual(aggregate.reports[0].source, "workspace-live-sync.json")
  assert.doesNotThrow(() => validateDrillValidationGateAggregate(aggregate))
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_providers=codex missing=none/)
})

test("fails aggregate requirements missing from otherwise passing reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedRequiredPresets: ["remote-home-extension"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["hosted-cloud-drills"],
      requiredFailureClassifications: ["remote-extension-sync"],
      requiredMatrices: ["remote-home-extension-matrix"],
      requiredMatrixClassifications: ["remote-extension-sync"],
      requiredDeploymentPresets: ["hosted-cloud"],
      requiredProviders: ["claude"],
      requiredScenarios: ["hetzner-collab"],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "failed")
  assert.equal(drillValidationGateAggregateExitCode(aggregate), 1)
  assert.deepEqual(aggregate.missingPresets, ["remote-home-extension"])
  assert.deepEqual(aggregate.missingProviders, ["claude"])
  assert.deepEqual(aggregate.missingScenarios, ["hetzner-collab"])
  assert.deepEqual(
    aggregate.nextActions.map(({ classification, nextAction }) => ({ classification, nextAction })),
    [
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring deployment presets: hosted-cloud",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring matrices: remote-home-extension-matrix",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring matrix classifications: remote-extension-sync",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring providers: claude",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring scenarios: hetzner-collab",
      },
      {
        classification: "platform-bundle",
        nextAction: "provide validation gate reports requiring failure classifications: remote-extension-sync",
      },
      {
        classification: "platform-bundle",
        nextAction: "provide validation gate reports requiring platform coverage areas: hosted-cloud-drills",
      },
      {
        classification: "validation-gate",
        nextAction: "provide validation gate reports for presets: remote-home-extension",
      },
    ],
  )
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_presets=remote-home-extension missing=remote-home-extension/)
})

test("rejects inconsistent aggregate status and coverage", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedRequiredPresets: ["workspace-live-sync"],
    validateReport: () => {},
  })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      status: "failed",
    }),
    /status does not match totals and requirements/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        presets: {},
      },
    }),
    /missingPresets does not match reports/,
  )
})

function reportFixture(overrides = {}) {
  return {
    schema: "arroba.drill.validation_gate.v1",
    status: "passed",
    presets: ["workspace-live-sync"],
    checks: {
      configuration: { status: "passed" },
      platformBundle: {
        status: "passed",
        requiredCoverageAreas: ["runtime-fixtures"],
        missingCoverageAreas: [],
        requiredFailureClassifications: ["kernel-authority"],
        missingFailureClassifications: [],
      },
      artifacts: { status: "skipped" },
      matrices: {
        status: "passed",
        requiredMatrices: ["workspace-live-sync-matrix"],
        missingMatrices: [],
        requiredMatrixClassifications: ["workspace-live-sync-conflict"],
        missingMatrixClassifications: [],
        requiredDeploymentPresets: ["local"],
        missingDeploymentPresets: [],
        requiredProviders: ["codex"],
        missingProviders: [],
        requiredScenarios: ["managed"],
        missingScenarios: [],
      },
      failures: { status: "skipped" },
    },
    nextActions: [],
    ...overrides,
  }
}
