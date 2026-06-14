import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  expandValidationGatePresetRequirements,
  normalizeRequiredDeploymentPresets,
  normalizeRequiredFailureClassifications,
  normalizeRequiredMatrices,
  normalizeRequiredMatrixClassifications,
  normalizeRequiredPlatformCoverageAreas,
  normalizeRequiredPresets,
  normalizeRequiredProviders,
  normalizeRequiredScenarios,
} from "./drill-validation-gate-presets.mjs"

test("describes stable validation gate presets", () => {
  assert.deepEqual(Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort(), [
    "remote-home-extension",
    "workspace-live-sync",
  ])
  assert.deepEqual(describeDrillValidationGatePresets().map((preset) => preset.name), [
    "remote-home-extension",
    "workspace-live-sync",
  ])
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["workspace-live-sync"] })[0].requiredMatrixClassifications,
    ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
  )
})

test("expands validation gate preset requirements", () => {
  assert.deepEqual(expandValidationGatePresetRequirements({
    presets: ["remote-home-extension"],
    requiredPlatformCoverageAreas: ["runtime-fixtures"],
    requiredFailureClassifications: ["workspace-live-sync-conflict"],
    requiredMatrices: ["custom-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local"],
  }), {
    requiredPlatformCoverageAreas: ["runtime-fixtures", "failure-diagnostics", "matrix-validation", "runtime-fixtures"],
    requiredFailureClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "worker-execution"],
    requiredMatrices: ["custom-matrix", "remote-home-extension-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "worker-execution"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local"],
  })
})

test("normalizes validation gate requirements", () => {
  assert.deepEqual(normalizeRequiredPresets(["workspace-live-sync,remote-home-extension", "workspace-live-sync"]), [
    "remote-home-extension",
    "workspace-live-sync",
  ])
  assert.deepEqual(normalizeRequiredPlatformCoverageAreas(["runtime-fixtures,matrix-validation"]), [
    "matrix-validation",
    "runtime-fixtures",
  ])
  assert.deepEqual(normalizeRequiredFailureClassifications(["kernel-authority,worker-execution"]), [
    "kernel-authority",
    "worker-execution",
  ])
  assert.deepEqual(normalizeRequiredMatrices(["b,a", "a"]), ["a", "b"])
  assert.deepEqual(normalizeRequiredMatrixClassifications(["remote-extension-sync,kernel-authority"]), [
    "kernel-authority",
    "remote-extension-sync",
  ])
  assert.deepEqual(normalizeRequiredDeploymentPresets(["local,hetzner"]), ["hetzner", "local"])
  assert.deepEqual(normalizeRequiredProviders(["opencode,codex", "codex"]), ["codex", "opencode"])
  assert.deepEqual(normalizeRequiredScenarios(["remote,local", "local"]), ["local", "remote"])
})

test("rejects unknown validation gate requirements", () => {
  assert.throws(
    () => normalizeRequiredPresets(["workspace-live-synch"]),
    /unknown validation gate preset: workspace-live-synch/,
  )
  assert.throws(
    () => normalizeRequiredFailureClassifications(["not-a-classification"]),
    /unknown required failure classification: not-a-classification/,
  )
  assert.throws(
    () => normalizeRequiredMatrixClassifications(["not-a-classification"]),
    /unknown required matrix classification: not-a-classification/,
  )
  assert.throws(
    () => normalizeRequiredDeploymentPresets(["not-a-preset"]),
    /unknown required deployment preset: not-a-preset/,
  )
})
