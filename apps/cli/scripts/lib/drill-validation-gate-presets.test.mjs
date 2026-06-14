import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  expandValidationGatePresetRequirements,
  normalizeRequiredDeploymentPresets,
  normalizeRequiredArtifactCoverageAreas,
  normalizeRequiredArtifactEvidenceRepos,
  normalizeRequiredArtifactKinds,
  normalizeRequiredArtifactSchemas,
  normalizeRequiredFailureClassifications,
  normalizeRequiredMatrices,
  normalizeRequiredMatrixClassifications,
  normalizeRequiredMatrixRuntimeSignals,
  normalizeRequiredPlatformCoverageAreas,
  normalizeRequiredPresets,
  normalizeRequiredProviders,
  normalizeRequiredRuntimeSignals,
  normalizeRequiredScenarios,
} from "./drill-validation-gate-presets.mjs"
import { workspaceLiveSyncRequiredScenarioIds } from "./workspace-live-sync-fixtures.mjs"

test("describes stable validation gate presets", () => {
  assert.deepEqual(Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort(), [
    "distributed-runtime",
    "native-provider-tui",
    "remote-agent-runtime",
    "remote-home-extension",
    "slice-runtime",
    "workspace-live-sync",
  ])
  assert.deepEqual(describeDrillValidationGatePresets().map((preset) => preset.name), [
    "distributed-runtime",
    "native-provider-tui",
    "remote-agent-runtime",
    "remote-home-extension",
    "slice-runtime",
    "workspace-live-sync",
  ])
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["distributed-runtime"] })[0],
    {
      name: "distributed-runtime",
      description: "End-to-end distributed runtime authority evidence across native TUI, remote agents, home extensions, slices, and Workspace Live Sync.",
      requiredPlatformCoverageAreas: ["failure-diagnostics", "matrix-validation", "runtime-fixtures"],
      requiredArtifactCoverageAreas: ["distributed-observability"],
      requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
      requiredArtifactKinds: ["validation-suite-run"],
      requiredArtifactEvidenceRepos: ["cloud", "oss"],
      requiredRuntimeSignals: ["agent-lifecycle", "client-projection-health", "home-extension-manifest-sync", "lease-health", "permission-interaction", "provider-run-lifecycle", "relay-target-freshness", "session-authority", "slice-auth-state", "slice-runtime-state", "workspace-live-sync-state"],
      requiredFailureClassifications: ["cloud-runtime", "docker-runtime", "kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution", "workspace-live-sync-conflict"],
      requiredMatrices: ["cloud-slice-runtime-matrix", "native-provider-tui-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["docker-runtime", "kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-extension-sync", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution", "workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["agent-lifecycle", "client-projection-health", "home-extension-manifest-sync", "lease-health", "permission-interaction", "provider-run-lifecycle", "relay-target-freshness", "session-authority", "slice-auth-state", "slice-runtime-state", "workspace-live-sync-state"],
      requiredDeploymentPresets: ["hetzner", "hosted-cloud", "local", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["agent-reuse", "collab-remote-agent", "hetzner-collab", "hetzner-single", "lease-reconnect", "local-collab", "local-managed-codex", "local-native-tui", "local-single", "local-tracked-codex", "permission-visibility", "provider-auth", "provider-run-binding", "remote-managed-codex", "remote-native-tui", "remote-prompt-dispatch", "remote-tracked-codex", "session-start", "single-user-remote-agent", "slice-lifecycle", "slice-native-tui", "transcript-parity", "ui-projection"],
    },
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["workspace-live-sync"] })[0],
    {
      name: "workspace-live-sync",
      description: "Workspace Live Sync local/remote matrix evidence and distributed sync diagnostics.",
      requiredPlatformCoverageAreas: ["failure-diagnostics", "matrix-validation", "runtime-fixtures"],
      requiredArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      requiredArtifactKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredRuntimeSignals: ["relay-target-freshness", "session-authority", "workspace-live-sync-state"],
      requiredFailureClassifications: ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["relay-target-freshness", "session-authority", "workspace-live-sync-state"],
      requiredDeploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["codex", "opencode"],
      requiredScenarios: workspaceLiveSyncRequiredScenarioIds(),
    },
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["slice-runtime"] })[0],
    {
      name: "slice-runtime",
      description: "Slice lifecycle, provider-auth isolation, worker discovery, and UI projection evidence.",
      requiredPlatformCoverageAreas: ["failure-diagnostics", "matrix-validation", "runtime-fixtures"],
      requiredArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      requiredArtifactKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredRuntimeSignals: ["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "session-authority", "slice-auth-state", "slice-runtime-state"],
      requiredFailureClassifications: ["cloud-runtime", "docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["cloud-slice-runtime-matrix", "slice-runtime-matrix"],
      requiredMatrixClassifications: ["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrixRuntimeSignals: ["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "session-authority", "slice-auth-state", "slice-runtime-state"],
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
      requiredArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      requiredArtifactKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredRuntimeSignals: ["client-projection-health", "permission-interaction", "provider-run-lifecycle", "session-authority"],
      requiredFailureClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["native-provider-tui-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrixRuntimeSignals: ["client-projection-health", "permission-interaction", "provider-run-lifecycle", "session-authority"],
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
      requiredArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      requiredArtifactKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredRuntimeSignals: ["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"],
      requiredFailureClassifications: ["cloud-runtime", "kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-host-capacity", "remote-worker-version", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["remote-agent-runtime-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "ui-client-projection", "worker-execution"],
      requiredMatrixRuntimeSignals: ["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"],
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
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    requiredFailureClassifications: ["workspace-live-sync-conflict"],
    requiredMatrices: ["custom-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict"],
    requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local"],
  }), {
    requiredPlatformCoverageAreas: ["runtime-fixtures", "failure-diagnostics", "matrix-validation", "runtime-fixtures"],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    requiredArtifactKinds: [],
    requiredArtifactEvidenceRepos: [],
    requiredRuntimeSignals: ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"],
    requiredFailureClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "worker-execution"],
    requiredMatrices: ["custom-matrix", "remote-home-extension-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "worker-execution"],
    requiredMatrixRuntimeSignals: ["workspace-live-sync-state", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local"],
  })
  assert.deepEqual(expandValidationGatePresetRequirements({
    presets: ["distributed-runtime"],
    requiredPlatformCoverageAreas: [],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
  }).requiredArtifactCoverageAreas, ["distributed-observability"])
  assert.deepEqual(expandValidationGatePresetRequirements({
    presets: ["distributed-runtime"],
    requiredPlatformCoverageAreas: [],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
  }).requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.deepEqual(expandValidationGatePresetRequirements({
    presets: ["distributed-runtime"],
    requiredPlatformCoverageAreas: [],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    requiredArtifactKinds: [],
    requiredArtifactEvidenceRepos: [],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
  }).requiredArtifactKinds, ["validation-suite-run"])
  assert.deepEqual(expandValidationGatePresetRequirements({
    presets: ["distributed-runtime"],
    requiredPlatformCoverageAreas: [],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    requiredArtifactKinds: [],
    requiredArtifactEvidenceRepos: [],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
  }).requiredArtifactEvidenceRepos, ["cloud", "oss"])
})

test("normalizes validation gate requirements", () => {
  assert.deepEqual(normalizeRequiredPresets(["workspace-live-sync,remote-home-extension", "slice-runtime,native-provider-tui", "remote-agent-runtime,distributed-runtime"]), [
    "distributed-runtime",
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
  assert.deepEqual(normalizeRequiredArtifactCoverageAreas(["distributed-observability,matrix-validation", "distributed-observability"]), [
    "distributed-observability",
    "matrix-validation",
  ])
  assert.deepEqual(normalizeRequiredArtifactSchemas(["arroba.drill.matrix.v1,arroba.drill.validation_suite_run.v1", "arroba.drill.matrix.v1"]), [
    "arroba.drill.matrix.v1",
    "arroba.drill.validation_suite_run.v1",
  ])
  assert.deepEqual(normalizeRequiredArtifactKinds(["validation-suite-run,matrix-report", "matrix-report"]), [
    "matrix-report",
    "validation-suite-run",
  ])
  assert.deepEqual(normalizeRequiredArtifactEvidenceRepos(["oss,cloud", "cloud"]), [
    "cloud",
    "oss",
  ])
  assert.deepEqual(normalizeRequiredRuntimeSignals(["session-authority,lease-health", "session-authority"]), [
    "lease-health",
    "session-authority",
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
  assert.deepEqual(normalizeRequiredMatrixRuntimeSignals(["workspace-live-sync-state,session-authority"]), [
    "session-authority",
    "workspace-live-sync-state",
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
    () => normalizeRequiredArtifactSchemas([""]),
    /requiredArtifactSchemas has invalid schema/,
  )
  assert.throws(
    () => normalizeRequiredFailureClassifications(["not-a-classification"]),
    /unknown required failure classification: not-a-classification/,
  )
  assert.throws(
    () => normalizeRequiredRuntimeSignals(["not-a-signal"]),
    /unknown required runtime signal: not-a-signal/,
  )
  assert.throws(
    () => normalizeRequiredMatrixClassifications(["not-a-classification"]),
    /unknown required matrix classification: not-a-classification/,
  )
  assert.throws(
    () => normalizeRequiredMatrixRuntimeSignals(["not-a-signal"]),
    /unknown required matrix runtime signal: not-a-signal/,
  )
  assert.throws(
    () => normalizeRequiredDeploymentPresets(["not-a-preset"]),
    /unknown required deployment preset: not-a-preset/,
  )
})
