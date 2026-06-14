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
    "native-provider-tui",
    "remote-agent-runtime",
    "remote-home-extension",
    "slice-runtime",
    "workspace-live-sync",
  ])
  assert.deepEqual(describeDrillValidationGatePresets().map((preset) => preset.name), [
    "native-provider-tui",
    "remote-agent-runtime",
    "remote-home-extension",
    "slice-runtime",
    "workspace-live-sync",
  ])
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["workspace-live-sync"] })[0].requiredMatrixClassifications,
    ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["slice-runtime"] })[0],
    {
      name: "slice-runtime",
      description: "Slice lifecycle, provider-auth isolation, worker discovery, and UI projection evidence.",
      requiredPlatformCoverageAreas: ["failure-diagnostics", "matrix-validation", "runtime-fixtures"],
      requiredFailureClassifications: ["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["slice-runtime-matrix"],
      requiredMatrixClassifications: ["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"],
      requiredDeploymentPresets: ["hosted-cloud", "local", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["agent-reuse", "provider-auth", "session-start", "slice-lifecycle", "ui-projection"],
    },
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["native-provider-tui"] })[0],
    {
      name: "native-provider-tui",
      description: "Native provider TUI parity across local, remote, slice, permissions, and UI projection paths.",
      requiredPlatformCoverageAreas: ["failure-diagnostics", "matrix-validation", "runtime-fixtures"],
      requiredFailureClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["native-provider-tui-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"],
      requiredDeploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["local-native-tui", "permission-visibility", "remote-native-tui", "slice-native-tui", "transcript-parity"],
    },
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["remote-agent-runtime"] })[0],
    {
      name: "remote-agent-runtime",
      description: "Leased remote-agent lifecycle, worker provider-run binding, relay freshness, and collab projection evidence.",
      requiredPlatformCoverageAreas: ["failure-diagnostics", "matrix-validation", "runtime-fixtures"],
      requiredFailureClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-host-capacity", "remote-worker-version", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["remote-agent-runtime-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "ui-client-projection", "worker-execution"],
      requiredDeploymentPresets: ["hetzner", "hosted-cloud", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["collab-remote-agent", "lease-reconnect", "provider-run-binding", "remote-prompt-dispatch", "single-user-remote-agent"],
    },
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
    requiredFailureClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "worker-execution"],
    requiredMatrices: ["custom-matrix", "remote-home-extension-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "worker-execution"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local"],
  })
})

test("normalizes validation gate requirements", () => {
  assert.deepEqual(normalizeRequiredPresets(["workspace-live-sync,remote-home-extension", "slice-runtime,native-provider-tui", "remote-agent-runtime"]), [
    "native-provider-tui",
    "remote-agent-runtime",
    "remote-home-extension",
    "slice-runtime",
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
