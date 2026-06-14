import assert from "node:assert/strict"
import test from "node:test"

import { configurationValidationGateCheck } from "./drill-validation-gate-configuration-check.mjs"

test("fails when no validation evidence or requirements are configured", () => {
  assert.deepEqual(configurationValidationGateCheck(configuration()), {
    status: "failed",
    error: "no validation checks configured",
  })
})

test("passes when artifact evidence is configured", () => {
  assert.deepEqual(configurationValidationGateCheck(configuration({
    artifactRoots: ["/tmp/artifacts"],
  })), { status: "passed" })
  assert.deepEqual(configurationValidationGateCheck(configuration({
    artifactIndexes: ["/tmp/artifacts/arroba-drill-artifacts.json"],
  })), { status: "passed" })
})

test("passes when failure evidence is configured", () => {
  assert.deepEqual(configurationValidationGateCheck(configuration({
    failureRoots: ["/tmp/failures"],
  })), { status: "passed" })
  assert.deepEqual(configurationValidationGateCheck(configuration({
    failureInputs: ["/tmp/failures/arroba-drill-failure.json"],
  })), { status: "passed" })
})

test("passes when matrix evidence is configured", () => {
  assert.deepEqual(configurationValidationGateCheck(configuration({
    matrixRoots: ["/tmp/matrices"],
  })), { status: "passed" })
  assert.deepEqual(configurationValidationGateCheck(configuration({
    matrixReports: ["/tmp/matrix.json"],
  })), { status: "passed" })
})

test("passes when platform evidence or aggregate requirements are configured", () => {
  const cases = [
    { platformBundleDir: "/tmp/platform" },
    { requiredPlatformCoverageAreas: ["matrix-validation"] },
    { requiredRuntimeSignals: ["lease-health"] },
    { requiredFailureClassifications: ["kernel-authority"] },
    { requiredMatrices: ["workspace-live-sync-matrix"] },
    { requiredMatrixClassifications: ["workspace-live-sync-conflict"] },
    { requiredMatrixRuntimeSignals: ["workspace-live-sync-state"] },
    { requiredDeploymentPresets: ["local"] },
    { requiredProviders: ["codex"] },
    { requiredScenarios: ["tracked"] },
  ]

  for (const overrides of cases) {
    assert.deepEqual(configurationValidationGateCheck(configuration(overrides)), { status: "passed" })
  }
})

function configuration(overrides = {}) {
  return {
    artifactIndexes: [],
    artifactRoots: [],
    failureInputs: [],
    failureRoots: [],
    matrixReports: [],
    matrixRoots: [],
    platformBundleDir: null,
    requiredPlatformCoverageAreas: [],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
    ...overrides,
  }
}
