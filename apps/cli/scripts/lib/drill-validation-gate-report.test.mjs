import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_GATE_SCHEMA,
  validateDrillValidationGateReport,
} from "./drill-validation-gate-report.mjs"

test("accepts a minimal passed validation gate report", () => {
  assert.doesNotThrow(() => validateDrillValidationGateReport(report()))
})

test("rejects reports with unsupported schema or mismatched top-level status", () => {
  assert.throws(
    () => validateDrillValidationGateReport({ ...report(), schema: "wrong" }),
    /unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport({
      ...report({ checks: { artifacts: { status: "failed", roots: [], inputs: [], indexPaths: [], error: "missing" } } }),
      status: "passed",
    }),
    /status does not match check statuses/,
  )
})

test("rejects invalid configuration and next-action records", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: { configuration: { status: "skipped" } },
    })),
    /checks\.configuration cannot be skipped/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      nextActions: [{ owner: "validation-harness", classification: "validation-gate", nextAction: "" }],
    })),
    /nextActions\[0\] is missing nextAction/,
  )
})

test("validates platform bundle summary evidence", () => {
  assert.doesNotThrow(() => validateDrillValidationGateReport(report({
    checks: {
      platformBundle: {
        status: "passed",
        dir: "/tmp/platform",
        requiredCoverageAreas: [],
        missingCoverageAreas: [],
        requiredFailureClassifications: [],
        missingFailureClassifications: [],
        artifacts: [{
          path: "validation-suite.json",
          schema: "arroba.drill.validation_suite.v1",
          sha256: "a".repeat(64),
          sizeBytes: 10,
        }],
        validationSuite: {
          testCount: 2,
          coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
        },
        failureTaxonomy: {
          drill: ["kernel-authority"],
          scenario: ["kernel-authority"],
        },
      },
    },
  })))
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        platformBundle: {
          status: "passed",
          dir: "/tmp/platform",
          requiredCoverageAreas: [],
          missingCoverageAreas: [],
          requiredFailureClassifications: [],
          missingFailureClassifications: [],
          artifacts: [{
            path: "validation-suite.json",
            schema: "arroba.drill.validation_suite.v1",
            sha256: "bad",
            sizeBytes: 10,
          }],
          validationSuite: {
            testCount: 2,
            coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
          },
        },
      },
    })),
    /artifacts\[0\] has invalid sha256/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        platformBundle: {
          status: "passed",
          dir: "/tmp/platform",
          requiredCoverageAreas: [],
          missingCoverageAreas: [],
          requiredFailureClassifications: [],
          missingFailureClassifications: [],
          artifacts: [],
          validationSuite: {
            testCount: 3,
            coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
          },
        },
      },
    })),
    /coverageAreas do not match testCount/,
  )
})

test("validates aggregate schemas for artifact, matrix, and failure checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          aggregate: { schema: "wrong" },
        },
      },
    })),
    /checks\.artifacts\.aggregate has unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          aggregate: { schema: "wrong" },
        },
      },
    })),
    /checks\.matrices\.aggregate has unsupported schema/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        failures: {
          status: "passed",
          roots: [],
          inputs: [],
          manifestPaths: [],
          aggregate: { schema: "wrong" },
        },
      },
    })),
    /checks\.failures\.aggregate has unsupported schema/,
  )
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
    matrices: matrixCheck(),
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

function matrixCheck(overrides = {}) {
  return {
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
    ...overrides,
  }
}
