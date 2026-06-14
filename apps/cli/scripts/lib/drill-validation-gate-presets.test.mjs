import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  expandValidationGatePresetRequirements,
  normalizeRequiredDeploymentPresets,
  normalizeRequiredArtifactCoverageAreas,
  normalizeRequiredArtifactClassifications,
  normalizeRequiredArtifactEvidenceRepos,
  normalizeRequiredArtifactKinds,
  normalizeRequiredArtifactOwners,
  normalizeRequiredArtifactRuntimeSignalOwners,
  normalizeRequiredArtifactRuntimeSignals,
  normalizeRequiredArtifactSchemas,
  normalizeRequiredFailureClassifications,
  normalizeRequiredGeneratedEvidenceKinds,
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
      requiredArtifactGeneratedEvidenceKinds: [],
      requiredArtifactEvidenceRepos: ["cloud", "oss"],
      requiredArtifactRuntimeSignals: ["agent-lifecycle", "client-projection-health", "home-extension-manifest-sync", "lease-health", "permission-interaction", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state", "workspace-live-sync-state"],
      requiredArtifactRuntimeSignalOwners: ["kernel-authority", "provider-account", "provider-runtime", "runtime-network", "runtime-state", "ui-client", "worker-kernel"],
      requiredArtifactOwners: ["validation-platform"],
      requiredArtifactClassifications: ["cloud-validation-suite", "validation-suite"],
      requiredRuntimeSignals: ["agent-lifecycle", "client-projection-health", "home-extension-manifest-sync", "lease-health", "permission-interaction", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state", "workspace-live-sync-state"],
      requiredFailureClassifications: ["cloud-runtime", "docker-runtime", "kernel-authority", "provider-auth", "provider-error", "projection-staleness", "relay-runtime", "relay-target-freshness", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution", "workspace-live-sync-conflict"],
      requiredMatrices: ["cloud-slice-runtime-matrix", "native-provider-tui-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["docker-runtime", "kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-extension-sync", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution", "workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["agent-lifecycle", "client-projection-health", "home-extension-manifest-sync", "lease-health", "permission-interaction", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state", "workspace-live-sync-state"],
      requiredDeploymentPresets: ["hetzner", "hosted-cloud", "local", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["agent-reuse", "collab-remote-agent", "hetzner-collab", "hetzner-single", "lease-reconnect", "local-collab", "local-managed-codex", "local-native-tui", "local-single", "local-tracked-codex", "permission-visibility", "provider-auth", "provider-run-binding", "remote-managed-codex", "remote-native-tui", "remote-prompt-dispatch", "remote-tracked-codex", "session-start", "single-user-remote-agent", "slice-lifecycle", "slice-native-tui", "transcript-parity", "ui-projection"],
      requiredGeneratedEvidenceKinds: [],
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
      requiredArtifactGeneratedEvidenceKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      requiredArtifactClassifications: [],
      requiredRuntimeSignals: ["relay-target-freshness", "session-authority", "workspace-live-sync-state"],
      requiredFailureClassifications: ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["relay-target-freshness", "session-authority", "workspace-live-sync-state"],
      requiredDeploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["codex", "opencode"],
      requiredScenarios: workspaceLiveSyncRequiredScenarioIds(),
      requiredGeneratedEvidenceKinds: [],
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
      requiredArtifactGeneratedEvidenceKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      requiredArtifactClassifications: [],
      requiredRuntimeSignals: ["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state"],
      requiredFailureClassifications: ["cloud-runtime", "docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["cloud-slice-runtime-matrix", "slice-runtime-matrix"],
      requiredMatrixClassifications: ["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrixRuntimeSignals: ["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state"],
      requiredDeploymentPresets: ["hosted-cloud", "local", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["agent-reuse", "provider-auth", "session-start", "slice-lifecycle", "ui-projection"],
      requiredGeneratedEvidenceKinds: [],
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
      requiredArtifactGeneratedEvidenceKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      requiredArtifactClassifications: [],
      requiredRuntimeSignals: ["client-projection-health", "permission-interaction", "provider-run-lifecycle", "runtime-projection-health", "session-authority"],
      requiredFailureClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["native-provider-tui-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"],
      requiredMatrixRuntimeSignals: ["client-projection-health", "permission-interaction", "provider-run-lifecycle", "runtime-projection-health", "session-authority"],
      requiredDeploymentPresets: ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["local-native-tui", "permission-visibility", "remote-native-tui", "slice-native-tui", "transcript-parity"],
      requiredGeneratedEvidenceKinds: [],
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
      requiredArtifactGeneratedEvidenceKinds: [],
      requiredArtifactEvidenceRepos: [],
      requiredArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      requiredArtifactClassifications: [],
      requiredRuntimeSignals: ["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority"],
      requiredFailureClassifications: ["cloud-runtime", "kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-host-capacity", "remote-worker-version", "ui-client-projection", "worker-execution"],
      requiredMatrices: ["remote-agent-runtime-matrix"],
      requiredMatrixClassifications: ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "ui-client-projection", "worker-execution"],
      requiredMatrixRuntimeSignals: ["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority"],
      requiredDeploymentPresets: ["hetzner", "hosted-cloud", "same-host-remote", "self-hosted-relay"],
      requiredProviders: ["claude", "codex", "opencode"],
      requiredScenarios: ["collab-remote-agent", "lease-reconnect", "provider-run-binding", "remote-prompt-dispatch", "single-user-remote-agent"],
      requiredGeneratedEvidenceKinds: [],
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
    requiredArtifactGeneratedEvidenceKinds: [],
    requiredArtifactEvidenceRepos: [],
    requiredArtifactRuntimeSignals: [],
    requiredArtifactRuntimeSignalOwners: [],
    requiredArtifactOwners: [],
    requiredArtifactClassifications: [],
    requiredRuntimeSignals: ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"],
    requiredFailureClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "worker-execution"],
    requiredMatrices: ["custom-matrix", "remote-home-extension-matrix"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict", "kernel-authority", "remote-extension-sync", "worker-execution"],
    requiredMatrixRuntimeSignals: ["workspace-live-sync-state", "home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"],
    requiredDeploymentPresets: ["local"],
    requiredProviders: ["codex"],
    requiredScenarios: ["local"],
    requiredGeneratedEvidenceKinds: [],
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
  assert.deepEqual(normalizeRequiredGeneratedEvidenceKinds(["validation-suite-run,matrix-report", "matrix-report"]), [
    "matrix-report",
    "validation-suite-run",
  ])
  assert.deepEqual(normalizeRequiredArtifactEvidenceRepos(["oss,cloud", "cloud"]), [
    "cloud",
    "oss",
  ])
  assert.deepEqual(normalizeRequiredArtifactRuntimeSignals(["workspace-live-sync-state,session-authority"]), [
    "session-authority",
    "workspace-live-sync-state",
  ])
  assert.deepEqual(normalizeRequiredArtifactRuntimeSignalOwners(["runtime-state,kernel-authority"]), [
    "kernel-authority",
    "runtime-state",
  ])
  assert.deepEqual(normalizeRequiredArtifactOwners(["validation-platform,runtime-network", "validation-platform"]), [
    "runtime-network",
    "validation-platform",
  ])
  assert.deepEqual(normalizeRequiredArtifactClassifications(["validation-gate,cloud-validation-suite"]), [
    "cloud-validation-suite",
    "validation-gate",
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
    () => normalizeRequiredArtifactKinds(["validation-sutie"]),
    /unknown required artifact kind: validation-sutie/,
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
    () => normalizeRequiredArtifactRuntimeSignals(["not-a-signal"]),
    /unknown required artifact runtime signal: not-a-signal/,
  )
  assert.throws(
    () => normalizeRequiredArtifactRuntimeSignalOwners(["not-an-owner"]),
    /unknown required artifact runtime signal owner: not-an-owner/,
  )
  assert.throws(
    () => normalizeRequiredArtifactEvidenceRepos(["cluod"]),
    /unknown required artifact evidence repo: cluod/,
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
  assert.throws(
    () => normalizeRequiredGeneratedEvidenceKinds(["not-generated"]),
    /unknown required generated evidence kind: not-generated/,
  )
})
