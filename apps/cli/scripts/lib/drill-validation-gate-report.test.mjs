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

test("validates optional generated evidence provenance", () => {
  assert.doesNotThrow(() => validateDrillValidationGateReport(report({
    generatedEvidence: generatedEvidence(),
  })))
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        validationSuites: {
          enabled: false,
          artifactIndexes: ["/tmp/artifacts.json"],
          outputRoots: [],
        },
      },
    })),
    /generatedEvidence\.validationSuites disabled evidence has paths/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        validationSuites: {
          enabled: true,
          artifactIndexes: [],
          outputRoots: [],
        },
      },
    })),
    /generatedEvidence\.validationSuites enabled evidence is missing paths/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          ...generatedEvidence().matrixReports,
          commands: [{
            args: [],
            artifactIndexPath: "/tmp/matrix-artifacts.json",
            cwd: "/repo/arroba",
            reportPath: "",
            scriptPath: "/repo/arroba/matrix.mjs",
          }],
        },
      },
    })),
    /generatedEvidence\.matrixReports\.commands\[0\] has invalid reportPath/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          enabled: true,
          roots: [],
          commands: [],
          dryRun: false,
          continueOnFailure: false,
        },
      },
    })),
    /generatedEvidence\.matrixReports enabled evidence is missing paths/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      generatedEvidence: {
        ...generatedEvidence(),
        matrixReports: {
          enabled: false,
          roots: ["/tmp/matrices"],
          commands: [],
          dryRun: false,
          continueOnFailure: false,
        },
      },
    })),
    /generatedEvidence\.matrixReports disabled evidence has paths/,
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
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          aggregate: { schema: "arroba.drill.artifact_index.aggregate.v1" },
        },
      },
    })),
    /checks\.artifacts\.aggregate has invalid totals/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        matrices: {
          ...matrixCheck(),
          aggregate: {
            ...matrixAggregate(),
            runtimeSignals: { "session-authority": 2 },
          },
        },
      },
    })),
    /checks\.matrices\.aggregate runtimeSignals do not match runtimeSignalScenarios/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        failures: {
          status: "failed",
          roots: [],
          inputs: [],
          manifestPaths: [],
          aggregate: {
            ...failureAggregate(),
            runtimeSignals: { "lease-health": 2 },
            runtimeSignalOwners: { "kernel-authority": 2 },
          },
        },
      },
    })),
    /checks\.failures\.aggregate runtimeSignals do not match failures/,
  )
})

test("rejects unknown artifact evidence repo labels in report checks", () => {
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactEvidenceRepos: ["cluod"],
          missingArtifactEvidenceRepos: [],
        },
      },
    })),
    /checks\.artifacts\.requiredArtifactEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateReport(report({
      checks: {
        artifacts: {
          status: "passed",
          roots: [],
          inputs: [],
          indexPaths: [],
          requiredArtifactEvidenceRepos: [],
          missingArtifactEvidenceRepos: ["cluod"],
        },
      },
    })),
    /checks\.artifacts\.missingArtifactEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
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

function generatedEvidence() {
  return {
    validationSuites: {
      enabled: true,
      artifactIndexes: [
        "/tmp/suites/cloud/arroba-drill-artifacts.json",
        "/tmp/suites/oss/arroba-drill-artifacts.json",
      ],
      outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
    },
    matrixReports: {
      enabled: true,
      roots: ["/tmp/matrices/cloud", "/tmp/matrices/oss"],
      commands: [{
        args: ["--include-hetzner"],
        artifactIndexPath: "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
        cwd: "/repo/arroba",
        reportPath: "/tmp/matrices/oss/native-provider-tui-matrix.json",
        scriptPath: "/repo/arroba/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
      }],
      dryRun: false,
      continueOnFailure: true,
    },
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

function matrixAggregate() {
  return {
    schema: "arroba.drill.matrix.aggregate.v1",
    status: "passed",
    totals: { reports: 1, scenarios: 1, passed: 1, failed: 0, skipped: 0, dryRun: 0, durationMs: 10 },
    failedScenarios: [],
    skippedScenarios: [],
    incompleteScenarios: [],
    owners: {},
    matrixNames: { "test-matrix": 1 },
    deploymentPresets: {},
    providers: {},
    scenarioIds: { local: 1 },
    runtimeSignals: { "session-authority": 1 },
    runtimeSignalScenarios: {
      "session-authority": [{
        matrix: "test-matrix",
        source: "/tmp/matrix.json",
        id: "local",
        status: "passed",
      }],
    },
    nextActions: [],
    reports: [{
      matrix: "test-matrix",
      source: "/tmp/matrix.json",
      status: "passed",
      deploymentPresets: [],
      providers: [],
      scenarioIds: ["local"],
      runtimeSignals: { "session-authority": 1 },
      runtimeSignalScenarios: {
        "session-authority": [{
          id: "local",
          status: "passed",
        }],
      },
      scenarioCount: 1,
      counts: { passed: 1, failed: 0, skipped: 0, dryRun: 0 },
      durationMs: 10,
    }],
  }
}

function failureAggregate() {
  return {
    schema: "arroba.drill.failure.aggregate.v1",
    total: 1,
    owners: { "runtime-network": 1 },
    classifications: { "relay-runtime": 1 },
    runtimeSignals: { "lease-health": 1 },
    runtimeSignalOwners: { "kernel-authority": 1 },
    nextActions: [{
      owner: "runtime-network",
      classification: "relay-runtime",
      nextAction: "inspect relay and kernel logs in the preserved artifact root, then rerun the drill",
      count: 1,
    }],
    failures: [{
      drill: "relay-drill",
      rootDir: "/tmp/failure",
      owner: "runtime-network",
      classification: "relay-runtime",
      runtimeSignals: ["lease-health"],
      nextAction: "inspect relay and kernel logs in the preserved artifact root, then rerun the drill",
    }],
  }
}
