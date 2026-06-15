import { isKnownDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import {
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_REPOS,
} from "./drill-distributed-runtime-evidence.mjs"
import { DRILL_DEPLOYMENT_PRESETS } from "./drill-environment-presets.mjs"
import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { isKnownDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import {
  validateDrillGeneratedEvidenceKind,
  validateDrillGeneratedEvidencePath,
} from "./drill-generated-evidence-metadata.mjs"
import { isKnownDrillGeneratedMatrixName } from "./drill-generated-matrix-names.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import {
  isKnownDrillProvider,
  parseProviderAccountAlias,
} from "./drill-provider-profiles.mjs"
import {
  DRILL_RUNTIME_SIGNAL_OWNERS,
  drillRuntimeSignalOwnersFor,
  isKnownDrillRuntimeSignal,
} from "./drill-runtime-signals.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"
import { WORKSPACE_LIVE_SYNC_REQUIRED_SCENARIO_IDS } from "./workspace-live-sync-fixtures.mjs"

export const DRILL_VALIDATION_GATE_PRESETS = Object.freeze({
  "distributed-runtime": Object.freeze({
    description: "End-to-end distributed runtime authority evidence across native TUI, remote agents, home extensions, slices, and Workspace Live Sync.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredArtifactCoverageAreas: Object.freeze(["distributed-observability"]),
    requiredArtifactSchemas: Object.freeze(["arroba.drill.validation_suite_run.v1"]),
    requiredArtifactKinds: Object.freeze(["validation-suite-run"]),
    requiredArtifactGeneratedMatrixNames: DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES,
    requiredArtifactGeneratedMatrixRepos: DISTRIBUTED_RUNTIME_GENERATED_MATRIX_REPOS,
    requiredArtifactEvidenceRepos: Object.freeze(["cloud", "oss"]),
    requiredArtifactValidationPresets: Object.freeze(["distributed-runtime"]),
    requiredArtifactRuntimeSignals: Object.freeze([
      "agent-lifecycle",
      "client-projection-health",
      "home-extension-manifest-sync",
      "lease-health",
      "permission-interaction",
      "provider-run-lifecycle",
      "relay-target-freshness",
      "runtime-projection-health",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
      "workspace-live-sync-state",
    ]),
    requiredArtifactRuntimeSignalOwners: Object.freeze([
      "kernel-authority",
      "provider-account",
      "provider-runtime",
      "runtime-network",
      "runtime-state",
      "ui-client",
      "worker-kernel",
    ]),
    requiredArtifactOwners: Object.freeze(["validation-platform"]),
    requiredArtifactClassifications: Object.freeze(["cloud-validation-suite", "validation-suite"]),
    requiredArtifactExitCriterionStatuses: Object.freeze(["satisfied"]),
    requiredRuntimeSignals: Object.freeze([
      "agent-lifecycle",
      "client-projection-health",
      "home-extension-manifest-sync",
      "lease-health",
      "permission-interaction",
      "provider-run-lifecycle",
      "relay-target-freshness",
      "runtime-projection-health",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
      "workspace-live-sync-state",
    ]),
    requiredFailureClassifications: Object.freeze([
      "cloud-runtime",
      "docker-runtime",
      "kernel-authority",
      "provider-auth",
      "provider-error",
      "projection-staleness",
      "relay-runtime",
      "relay-target-freshness",
      "remote-extension-sync",
      "remote-host-capacity",
      "remote-worker-version",
      "slice-auth",
      "slice-runtime",
      "ui-client-projection",
      "worker-execution",
      "workspace-live-sync-conflict",
    ]),
    requiredMatrices: Object.freeze([
      "cloud-slice-runtime-matrix",
      "native-provider-tui-matrix",
      "remote-agent-runtime-matrix",
      "remote-home-extension-matrix",
      "slice-runtime-matrix",
      "workspace-live-sync-matrix",
    ]),
    requiredMatrixClassifications: Object.freeze([
      "docker-runtime",
      "kernel-authority",
      "provider-auth",
      "provider-error",
      "relay-runtime",
      "relay-target-freshness",
      "remote-extension-sync",
      "slice-auth",
      "slice-runtime",
      "ui-client-projection",
      "worker-execution",
      "workspace-live-sync-conflict",
    ]),
    requiredMatrixRuntimeSignals: Object.freeze([
      "agent-lifecycle",
      "client-projection-health",
      "home-extension-manifest-sync",
      "lease-health",
      "permission-interaction",
      "provider-run-lifecycle",
      "relay-target-freshness",
      "runtime-projection-health",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
      "workspace-live-sync-state",
    ]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "hosted-cloud", "local", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze([
      "agent-reuse",
      "collab-remote-agent",
      "hetzner-collab",
      "hetzner-collab-remote-agent",
      "hetzner-single",
      "hetzner-single-user-remote-agent",
      "hosted-collab-remote-agent",
      "hosted-single-user-remote-agent",
      "lease-reconnect",
      "local-collab",
      "local-managed-codex",
      "local-native-tui",
      "local-single",
      "local-tracked-codex",
      "permission-visibility",
      "provider-auth",
      "provider-run-binding",
      "remote-managed-codex",
      "remote-native-tui",
      "remote-prompt-dispatch",
      "remote-tracked-codex",
      "session-start",
      "single-user-remote-agent",
      "slice-lifecycle",
      "slice-native-tui",
      "transcript-parity",
      "ui-projection",
    ]),
  }),
  "native-provider-tui": Object.freeze({
    description: "Native provider TUI parity across local, remote, slice, permissions, and UI projection paths.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["client-projection-health", "permission-interaction", "provider-run-lifecycle", "runtime-projection-health", "session-authority"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"]),
    requiredMatrices: Object.freeze(["native-provider-tui-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"]),
    requiredMatrixRuntimeSignals: Object.freeze(["client-projection-health", "permission-interaction", "provider-run-lifecycle", "runtime-projection-health", "session-authority"]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "local", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze(["local-native-tui", "permission-visibility", "remote-native-tui", "slice-native-tui", "transcript-parity"]),
  }),
  "remote-agent-runtime": Object.freeze({
    description: "Leased remote-agent lifecycle, worker provider-run binding, relay freshness, and collab projection evidence.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority"]),
    requiredFailureClassifications: Object.freeze(["cloud-runtime", "kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-host-capacity", "remote-worker-version", "ui-client-projection", "worker-execution"]),
    requiredMatrices: Object.freeze(["remote-agent-runtime-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "ui-client-projection", "worker-execution"]),
    requiredMatrixRuntimeSignals: Object.freeze(["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "runtime-projection-health", "session-authority"]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "hosted-cloud", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze([
      "collab-remote-agent",
      "hetzner-collab-remote-agent",
      "hetzner-single-user-remote-agent",
      "hosted-collab-remote-agent",
      "hosted-single-user-remote-agent",
      "lease-reconnect",
      "provider-run-binding",
      "remote-prompt-dispatch",
      "single-user-remote-agent",
    ]),
  }),
  "workspace-live-sync": Object.freeze({
    description: "Workspace Live Sync local/remote matrix evidence and distributed sync diagnostics.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"]),
    requiredMatrices: Object.freeze(["workspace-live-sync-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"]),
    requiredMatrixRuntimeSignals: Object.freeze(["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "local", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["codex", "opencode"]),
    requiredScenarios: WORKSPACE_LIVE_SYNC_REQUIRED_SCENARIO_IDS,
  }),
  "remote-home-extension": Object.freeze({
    description: "Home-owned extension execution evidence for remote agents and collab authority checks.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "worker-execution"]),
    requiredMatrices: Object.freeze(["remote-home-extension-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "remote-extension-sync", "worker-execution"]),
    requiredMatrixRuntimeSignals: Object.freeze(["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "local", "self-hosted-relay"]),
    requiredScenarios: Object.freeze(["hetzner-collab", "hetzner-single", "local-collab", "local-single"]),
  }),
  "slice-runtime": Object.freeze({
    description: "Slice lifecycle, provider-auth isolation, worker discovery, and UI projection evidence.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state"]),
    requiredFailureClassifications: Object.freeze(["cloud-runtime", "docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"]),
    requiredMatrices: Object.freeze(["cloud-slice-runtime-matrix", "slice-runtime-matrix"]),
    requiredMatrixClassifications: Object.freeze(["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"]),
    requiredMatrixRuntimeSignals: Object.freeze(["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "runtime-projection-health", "session-authority", "slice-auth-state", "slice-runtime-state"]),
    requiredDeploymentPresets: Object.freeze(["hosted-cloud", "local", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze(["agent-reuse", "provider-auth", "session-start", "slice-lifecycle", "ui-projection"]),
  }),
})

export const DRILL_ARTIFACT_VALIDATION_PRESETS = Object.freeze([
  ...Object.keys(DRILL_VALIDATION_GATE_PRESETS),
  "cloud-distributed-runtime",
].sort())

export function describeDrillValidationGatePresets({ names = null } = {}) {
  const presetNames = names == null
    ? Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort()
    : normalizeRequiredPresets(Array.isArray(names) ? names : [names])
  return presetNames.map((name) => {
    const preset = DRILL_VALIDATION_GATE_PRESETS[name]
    return {
      name,
      description: preset.description,
      requiredPlatformCoverageAreas: [...(preset.requiredPlatformCoverageAreas ?? [])],
      requiredArtifactCoverageAreas: [...(preset.requiredArtifactCoverageAreas ?? [])],
      requiredArtifactSchemas: [...(preset.requiredArtifactSchemas ?? [])],
      requiredArtifactKinds: [...(preset.requiredArtifactKinds ?? [])],
      requiredArtifactGeneratedEvidenceKinds: [...(preset.requiredArtifactGeneratedEvidenceKinds ?? [])],
      requiredArtifactGeneratedMatrixArtifactIndexes: [...(preset.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])],
      requiredArtifactGeneratedMatrixLimitations: [...(preset.requiredArtifactGeneratedMatrixLimitations ?? [])],
      requiredArtifactGeneratedMatrixNames: [...(preset.requiredArtifactGeneratedMatrixNames ?? [])],
      requiredArtifactGeneratedMatrixRepos: [...(preset.requiredArtifactGeneratedMatrixRepos ?? [])],
      requiredArtifactEvidenceRepos: [...(preset.requiredArtifactEvidenceRepos ?? [])],
      requiredArtifactProviderAccountAliases: [...(preset.requiredArtifactProviderAccountAliases ?? [])],
      requiredArtifactValidationPresets: [...(preset.requiredArtifactValidationPresets ?? [])],
      requiredArtifactRuntimeSignals: [...(preset.requiredArtifactRuntimeSignals ?? [])],
      requiredArtifactRuntimeSignalOwners: [...(preset.requiredArtifactRuntimeSignalOwners ?? [])],
      requiredArtifactOwners: [...(preset.requiredArtifactOwners ?? [])],
      requiredArtifactClassifications: [...(preset.requiredArtifactClassifications ?? [])],
      requiredArtifactPlannedOwners: [...(preset.requiredArtifactPlannedOwners ?? [])],
      requiredArtifactPlannedClassifications: [...(preset.requiredArtifactPlannedClassifications ?? [])],
      requiredArtifactExitCriterionStatuses: [...(preset.requiredArtifactExitCriterionStatuses ?? [])],
      requiredArtifactIncompleteExitCriterionStatuses: [...(preset.requiredArtifactIncompleteExitCriterionStatuses ?? [])],
      requiredRuntimeSignals: [...(preset.requiredRuntimeSignals ?? [])],
      requiredRuntimeSignalOwners: presetRuntimeSignalOwners(preset),
      requiredFailureClassifications: [...(preset.requiredFailureClassifications ?? [])],
      requiredMatrices: [...(preset.requiredMatrices ?? [])],
      requiredMatrixClassifications: [...(preset.requiredMatrixClassifications ?? [])],
      requiredMatrixRuntimeSignals: [...(preset.requiredMatrixRuntimeSignals ?? [])],
      requiredDeploymentPresets: [...(preset.requiredDeploymentPresets ?? [])],
      requiredProviders: [...(preset.requiredProviders ?? [])],
      requiredScenarios: [...(preset.requiredScenarios ?? [])],
      requiredGeneratedEvidenceKinds: [...(preset.requiredGeneratedEvidenceKinds ?? [])],
      requiredGeneratedMatrixArtifactIndexes: [...(preset.requiredGeneratedMatrixArtifactIndexes ?? [])],
      requiredGeneratedMatrixLimitations: [...(preset.requiredGeneratedMatrixLimitations ?? [])],
      requiredGeneratedValidationSuiteArtifactIndexes: [...(preset.requiredGeneratedValidationSuiteArtifactIndexes ?? [])],
      requiredGeneratedValidationSuiteFailureRoots: [...(preset.requiredGeneratedValidationSuiteFailureRoots ?? [])],
    }
  })
}

export function expandValidationGatePresetRequirements({
  presets,
  requiredPlatformCoverageAreas,
  requiredArtifactCoverageAreas = [],
  requiredArtifactSchemas = [],
  requiredArtifactKinds = [],
  requiredArtifactGeneratedEvidenceKinds = [],
  requiredArtifactGeneratedMatrixArtifactIndexes = [],
  requiredArtifactGeneratedMatrixLimitations = [],
  requiredArtifactGeneratedMatrixNames = [],
  requiredArtifactGeneratedMatrixRepos = [],
  requiredArtifactEvidenceRepos = [],
  requiredArtifactProviderAccountAliases = [],
  requiredArtifactValidationPresets = [],
  requiredArtifactRuntimeSignals = [],
  requiredArtifactRuntimeSignalOwners = [],
  requiredArtifactOwners = [],
  requiredArtifactClassifications = [],
  requiredArtifactPlannedOwners = [],
  requiredArtifactPlannedClassifications = [],
  requiredArtifactExitCriterionStatuses = [],
  requiredArtifactIncompleteExitCriterionStatuses = [],
  requiredRuntimeSignals = [],
  requiredRuntimeSignalOwners = [],
  requiredFailureClassifications,
  requiredMatrices,
  requiredMatrixClassifications,
  requiredMatrixRuntimeSignals = [],
  requiredDeploymentPresets,
  requiredProviders,
  requiredScenarios,
  requiredGeneratedEvidenceKinds = [],
  requiredGeneratedMatrixArtifactIndexes = [],
  requiredGeneratedMatrixLimitations = [],
  requiredGeneratedValidationSuiteArtifactIndexes = [],
  requiredGeneratedValidationSuiteFailureRoots = [],
}) {
  const expanded = {
    requiredPlatformCoverageAreas: [...requiredPlatformCoverageAreas],
    requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
    requiredArtifactSchemas: [...requiredArtifactSchemas],
    requiredArtifactKinds: [...requiredArtifactKinds],
    requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
    requiredArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
    requiredArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
    requiredArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
    requiredArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
    requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
    requiredArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
    requiredArtifactValidationPresets: [...requiredArtifactValidationPresets],
    requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
    requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
    requiredArtifactOwners: [...requiredArtifactOwners],
    requiredArtifactClassifications: [...requiredArtifactClassifications],
    requiredArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
    requiredArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
    requiredArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
    requiredArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
    requiredRuntimeSignals: [...requiredRuntimeSignals],
    requiredRuntimeSignalOwners: [...requiredRuntimeSignalOwners],
    requiredFailureClassifications: [...requiredFailureClassifications],
    requiredMatrices: [...requiredMatrices],
    requiredMatrixClassifications: [...requiredMatrixClassifications],
    requiredMatrixRuntimeSignals: [...requiredMatrixRuntimeSignals],
    requiredDeploymentPresets: [...requiredDeploymentPresets],
    requiredProviders: [...requiredProviders],
    requiredScenarios: [...requiredScenarios],
    requiredGeneratedEvidenceKinds: [...requiredGeneratedEvidenceKinds],
    requiredGeneratedMatrixArtifactIndexes: [...requiredGeneratedMatrixArtifactIndexes],
    requiredGeneratedMatrixLimitations: [...requiredGeneratedMatrixLimitations],
    requiredGeneratedValidationSuiteArtifactIndexes: [...requiredGeneratedValidationSuiteArtifactIndexes],
    requiredGeneratedValidationSuiteFailureRoots: [...requiredGeneratedValidationSuiteFailureRoots],
  }
  for (const presetName of presets) {
    const preset = DRILL_VALIDATION_GATE_PRESETS[presetName]
    expanded.requiredPlatformCoverageAreas.push(...(preset.requiredPlatformCoverageAreas ?? []))
    expanded.requiredArtifactCoverageAreas.push(...(preset.requiredArtifactCoverageAreas ?? []))
    expanded.requiredArtifactSchemas.push(...(preset.requiredArtifactSchemas ?? []))
    expanded.requiredArtifactKinds.push(...(preset.requiredArtifactKinds ?? []))
    expanded.requiredArtifactGeneratedEvidenceKinds.push(...(preset.requiredArtifactGeneratedEvidenceKinds ?? []))
    expanded.requiredArtifactGeneratedMatrixArtifactIndexes.push(...(preset.requiredArtifactGeneratedMatrixArtifactIndexes ?? []))
    expanded.requiredArtifactGeneratedMatrixLimitations.push(...(preset.requiredArtifactGeneratedMatrixLimitations ?? []))
    expanded.requiredArtifactGeneratedMatrixNames.push(...(preset.requiredArtifactGeneratedMatrixNames ?? []))
    expanded.requiredArtifactGeneratedMatrixRepos.push(...(preset.requiredArtifactGeneratedMatrixRepos ?? []))
    expanded.requiredArtifactEvidenceRepos.push(...(preset.requiredArtifactEvidenceRepos ?? []))
    expanded.requiredArtifactProviderAccountAliases.push(...(preset.requiredArtifactProviderAccountAliases ?? []))
    expanded.requiredArtifactValidationPresets.push(...(preset.requiredArtifactValidationPresets ?? []))
    expanded.requiredArtifactRuntimeSignals.push(...(preset.requiredArtifactRuntimeSignals ?? []))
    expanded.requiredArtifactRuntimeSignalOwners.push(...(preset.requiredArtifactRuntimeSignalOwners ?? []))
    expanded.requiredArtifactOwners.push(...(preset.requiredArtifactOwners ?? []))
    expanded.requiredArtifactClassifications.push(...(preset.requiredArtifactClassifications ?? []))
    expanded.requiredArtifactPlannedOwners.push(...(preset.requiredArtifactPlannedOwners ?? []))
    expanded.requiredArtifactPlannedClassifications.push(...(preset.requiredArtifactPlannedClassifications ?? []))
    expanded.requiredArtifactExitCriterionStatuses.push(...(preset.requiredArtifactExitCriterionStatuses ?? []))
    expanded.requiredArtifactIncompleteExitCriterionStatuses.push(...(preset.requiredArtifactIncompleteExitCriterionStatuses ?? []))
    expanded.requiredRuntimeSignals.push(...(preset.requiredRuntimeSignals ?? []))
    expanded.requiredRuntimeSignalOwners.push(...presetRuntimeSignalOwners(preset))
    expanded.requiredFailureClassifications.push(...(preset.requiredFailureClassifications ?? []))
    expanded.requiredMatrices.push(...(preset.requiredMatrices ?? []))
    expanded.requiredMatrixClassifications.push(...(preset.requiredMatrixClassifications ?? []))
    expanded.requiredMatrixRuntimeSignals.push(...(preset.requiredMatrixRuntimeSignals ?? []))
    expanded.requiredDeploymentPresets.push(...(preset.requiredDeploymentPresets ?? []))
    expanded.requiredProviders.push(...(preset.requiredProviders ?? []))
    expanded.requiredScenarios.push(...(preset.requiredScenarios ?? []))
    expanded.requiredGeneratedEvidenceKinds.push(...(preset.requiredGeneratedEvidenceKinds ?? []))
    expanded.requiredGeneratedMatrixArtifactIndexes.push(...(preset.requiredGeneratedMatrixArtifactIndexes ?? []))
    expanded.requiredGeneratedMatrixLimitations.push(...(preset.requiredGeneratedMatrixLimitations ?? []))
    expanded.requiredGeneratedValidationSuiteArtifactIndexes.push(...(preset.requiredGeneratedValidationSuiteArtifactIndexes ?? []))
    expanded.requiredGeneratedValidationSuiteFailureRoots.push(...(preset.requiredGeneratedValidationSuiteFailureRoots ?? []))
  }
  return expanded
}

function presetRuntimeSignalOwners(preset) {
  if (preset.requiredRuntimeSignalOwners !== undefined) {
    return [...preset.requiredRuntimeSignalOwners]
  }
  return drillRuntimeSignalOwnersFor(preset.requiredRuntimeSignals ?? [])
}

export function normalizeRequiredArtifactCoverageAreas(requiredArtifactCoverageAreas) {
  if (!Array.isArray(requiredArtifactCoverageAreas)) {
    throw new Error("requiredArtifactCoverageAreas must be an array")
  }
  const areas = []
  for (const area of requiredArtifactCoverageAreas) {
    if (!nonEmptyString(area)) {
      throw new Error("requiredArtifactCoverageAreas has invalid area")
    }
    for (const value of area.split(",")) {
      const normalized = value.trim()
      if (normalized) areas.push(normalized)
    }
  }
  return [...new Set(areas)].sort()
}

export function normalizeRequiredArtifactSchemas(requiredArtifactSchemas) {
  if (!Array.isArray(requiredArtifactSchemas)) {
    throw new Error("requiredArtifactSchemas must be an array")
  }
  const schemas = []
  for (const schema of requiredArtifactSchemas) {
    if (!nonEmptyString(schema)) {
      throw new Error("requiredArtifactSchemas has invalid schema")
    }
    for (const value of schema.split(",")) {
      const normalized = value.trim()
      if (normalized) schemas.push(normalized)
    }
  }
  return [...new Set(schemas)].sort()
}

export function normalizeRequiredArtifactKinds(requiredArtifactKinds) {
  const kinds = normalizeCommaSeparatedStrings(requiredArtifactKinds, {
    fieldName: "requiredArtifactKinds",
    itemName: "kind",
  })
  for (const kind of kinds) {
    if (!isKnownDrillArtifactKind(kind)) {
      throw new Error(`unknown required artifact kind: ${kind}`)
    }
  }
  return kinds
}

export function normalizeRequiredGeneratedEvidenceKinds(requiredGeneratedEvidenceKinds) {
  const kinds = normalizeCommaSeparatedStrings(requiredGeneratedEvidenceKinds, {
    fieldName: "requiredGeneratedEvidenceKinds",
    itemName: "kind",
  })
  for (const kind of kinds) {
    try {
      validateDrillGeneratedEvidenceKind(kind, "requiredGeneratedEvidenceKinds")
    } catch {
      throw new Error(`unknown required generated evidence kind: ${kind}`)
    }
  }
  return kinds
}

export function normalizeRequiredGeneratedMatrixLimitations(requiredGeneratedMatrixLimitations) {
  const limitations = normalizeCommaSeparatedStrings(requiredGeneratedMatrixLimitations, {
    fieldName: "requiredGeneratedMatrixLimitations",
    itemName: "limitation",
  })
  for (const limitation of limitations) {
    try {
      validateDrillGeneratedMatrixLimitation(limitation, "requiredGeneratedMatrixLimitations")
    } catch {
      throw new Error(`unknown required generated matrix limitation: ${limitation}`)
    }
  }
  return limitations
}

export function normalizeRequiredGeneratedMatrixArtifactIndexes(requiredGeneratedMatrixArtifactIndexes) {
  return normalizeGeneratedEvidencePathRequirements(requiredGeneratedMatrixArtifactIndexes, {
    fieldName: "requiredGeneratedMatrixArtifactIndexes",
    itemName: "path",
  })
}

export function normalizeRequiredGeneratedValidationSuiteFailureRoots(requiredGeneratedValidationSuiteFailureRoots) {
  return normalizeGeneratedEvidencePathRequirements(requiredGeneratedValidationSuiteFailureRoots, {
    fieldName: "requiredGeneratedValidationSuiteFailureRoots",
    itemName: "root",
  })
}

export function normalizeRequiredGeneratedValidationSuiteArtifactIndexes(requiredGeneratedValidationSuiteArtifactIndexes) {
  return normalizeGeneratedEvidencePathRequirements(requiredGeneratedValidationSuiteArtifactIndexes, {
    fieldName: "requiredGeneratedValidationSuiteArtifactIndexes",
    itemName: "path",
  })
}

export function normalizeRequiredArtifactGeneratedMatrixArtifactIndexes(requiredArtifactGeneratedMatrixArtifactIndexes) {
  return normalizeGeneratedEvidencePathRequirements(requiredArtifactGeneratedMatrixArtifactIndexes, {
    fieldName: "requiredArtifactGeneratedMatrixArtifactIndexes",
    itemName: "path",
  })
}

export function normalizeRequiredArtifactGeneratedEvidenceKinds(requiredArtifactGeneratedEvidenceKinds) {
  const kinds = normalizeCommaSeparatedStrings(requiredArtifactGeneratedEvidenceKinds, {
    fieldName: "requiredArtifactGeneratedEvidenceKinds",
    itemName: "kind",
  })
  for (const kind of kinds) {
    try {
      validateDrillGeneratedEvidenceKind(kind, "requiredArtifactGeneratedEvidenceKinds")
    } catch {
      throw new Error(`unknown required artifact generated evidence kind: ${kind}`)
    }
  }
  return kinds
}

export function normalizeRequiredArtifactGeneratedMatrixLimitations(requiredArtifactGeneratedMatrixLimitations) {
  const limitations = normalizeCommaSeparatedStrings(requiredArtifactGeneratedMatrixLimitations, {
    fieldName: "requiredArtifactGeneratedMatrixLimitations",
    itemName: "limitation",
  })
  for (const limitation of limitations) {
    try {
      validateDrillGeneratedMatrixLimitation(limitation, "requiredArtifactGeneratedMatrixLimitations")
    } catch {
      throw new Error(`unknown required artifact generated matrix limitation: ${limitation}`)
    }
  }
  return limitations
}

export function normalizeRequiredArtifactGeneratedMatrixNames(requiredArtifactGeneratedMatrixNames) {
  const matrixNames = normalizeDiagnosticTextRequirements(requiredArtifactGeneratedMatrixNames, {
    fieldName: "requiredArtifactGeneratedMatrixNames",
    itemName: "matrix",
  })
  for (const matrixName of matrixNames) {
    if (!isKnownDrillGeneratedMatrixName(matrixName)) {
      throw new Error(`unknown required artifact generated matrix name: ${matrixName}`)
    }
  }
  return matrixNames
}

export function normalizeRequiredArtifactGeneratedMatrixRepos(requiredArtifactGeneratedMatrixRepos) {
  const repos = normalizeCommaSeparatedStrings(requiredArtifactGeneratedMatrixRepos, {
    fieldName: "requiredArtifactGeneratedMatrixRepos",
    itemName: "repo",
  })
  for (const repo of repos) {
    validateDrillArtifactEvidenceRepo(repo, "required artifact generated matrix repos", {
      message: () => `unknown required artifact generated matrix repo: ${repo}`,
    })
  }
  return repos
}

export function normalizeRequiredArtifactEvidenceRepos(requiredArtifactEvidenceRepos) {
  const repos = normalizeCommaSeparatedStrings(requiredArtifactEvidenceRepos, {
    fieldName: "requiredArtifactEvidenceRepos",
    itemName: "repo",
  })
  for (const repo of repos) {
    validateDrillArtifactEvidenceRepo(repo, "required artifact evidence repos", {
      message: () => `unknown required artifact evidence repo: ${repo}`,
    })
  }
  return repos
}

export function normalizeRequiredArtifactProviderAccountAliases(requiredArtifactProviderAccountAliases) {
  const aliases = normalizeCommaSeparatedStrings(requiredArtifactProviderAccountAliases, {
    fieldName: "requiredArtifactProviderAccountAliases",
    itemName: "alias",
  })
  for (const alias of aliases) {
    const { provider } = parseProviderAccountAlias(alias)
    if (!isKnownDrillProvider(provider)) {
      throw new Error(`unknown required artifact provider account alias provider: ${provider}`)
    }
  }
  return aliases
}

export function normalizeRequiredArtifactValidationPresets(requiredArtifactValidationPresets) {
  const presets = normalizeCommaSeparatedStrings(requiredArtifactValidationPresets, {
    fieldName: "requiredArtifactValidationPresets",
    itemName: "preset",
  })
  for (const preset of presets) {
    if (!isKnownDrillArtifactValidationPreset(preset)) {
      throw new Error(`unknown required artifact validation preset: ${preset}`)
    }
  }
  return presets
}

export function normalizeRequiredArtifactRuntimeSignals(requiredArtifactRuntimeSignals) {
  const signals = normalizeCommaSeparatedStrings(requiredArtifactRuntimeSignals, {
    fieldName: "requiredArtifactRuntimeSignals",
    itemName: "signal",
  })
  for (const signal of signals) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`unknown required artifact runtime signal: ${signal}`)
    }
  }
  return signals
}

export function normalizeRequiredArtifactRuntimeSignalOwners(requiredArtifactRuntimeSignalOwners) {
  const owners = normalizeCommaSeparatedStrings(requiredArtifactRuntimeSignalOwners, {
    fieldName: "requiredArtifactRuntimeSignalOwners",
    itemName: "owner",
  })
  for (const owner of owners) {
    if (!DRILL_RUNTIME_SIGNAL_OWNERS.includes(owner)) {
      throw new Error(`unknown required artifact runtime signal owner: ${owner}`)
    }
  }
  return owners
}

export function normalizeRequiredRuntimeSignalOwners(requiredRuntimeSignalOwners) {
  const owners = normalizeCommaSeparatedStrings(requiredRuntimeSignalOwners, {
    fieldName: "requiredRuntimeSignalOwners",
    itemName: "owner",
  })
  for (const owner of owners) {
    if (!DRILL_RUNTIME_SIGNAL_OWNERS.includes(owner)) {
      throw new Error(`unknown required runtime signal owner: ${owner}`)
    }
  }
  return owners
}

export function normalizeRequiredArtifactOwners(requiredArtifactOwners) {
  return normalizeCommaSeparatedStrings(requiredArtifactOwners, {
    fieldName: "requiredArtifactOwners",
    itemName: "owner",
  })
}

export function normalizeRequiredArtifactClassifications(requiredArtifactClassifications) {
  return normalizeCommaSeparatedStrings(requiredArtifactClassifications, {
    fieldName: "requiredArtifactClassifications",
    itemName: "classification",
  })
}

export function normalizeRequiredArtifactPlannedOwners(requiredArtifactPlannedOwners) {
  return normalizeDiagnosticTextRequirements(requiredArtifactPlannedOwners, {
    fieldName: "requiredArtifactPlannedOwners",
    itemName: "owner",
  })
}

export function normalizeRequiredArtifactPlannedClassifications(requiredArtifactPlannedClassifications) {
  return normalizeDiagnosticTextRequirements(requiredArtifactPlannedClassifications, {
    fieldName: "requiredArtifactPlannedClassifications",
    itemName: "classification",
  })
}

export function normalizeRequiredArtifactExitCriterionStatuses(requiredArtifactExitCriterionStatuses) {
  const statuses = normalizeCommaSeparatedStrings(requiredArtifactExitCriterionStatuses, {
    fieldName: "requiredArtifactExitCriterionStatuses",
    itemName: "status",
  })
  validateExitCriterionStatuses(statuses, "requiredArtifactExitCriterionStatuses")
  return statuses
}

export function normalizeRequiredArtifactIncompleteExitCriterionStatuses(requiredArtifactIncompleteExitCriterionStatuses) {
  const statuses = normalizeCommaSeparatedStrings(requiredArtifactIncompleteExitCriterionStatuses, {
    fieldName: "requiredArtifactIncompleteExitCriterionStatuses",
    itemName: "status",
  })
  validateExitCriterionStatuses(statuses, "requiredArtifactIncompleteExitCriterionStatuses")
  return statuses
}

export function normalizeRequiredRuntimeSignals(requiredRuntimeSignals) {
  if (!Array.isArray(requiredRuntimeSignals)) {
    throw new Error("requiredRuntimeSignals must be an array")
  }
  const signals = []
  for (const signal of requiredRuntimeSignals) {
    if (!nonEmptyString(signal)) {
      throw new Error("requiredRuntimeSignals has invalid signal")
    }
    for (const value of signal.split(",")) {
      const normalized = value.trim()
      if (normalized) signals.push(normalized)
    }
  }
  const normalizedSignals = [...new Set(signals)].sort()
  for (const signal of normalizedSignals) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`unknown required runtime signal: ${signal}`)
    }
  }
  return normalizedSignals
}

export function normalizeRequiredPlatformCoverageAreas(requiredPlatformCoverageAreas) {
  if (!Array.isArray(requiredPlatformCoverageAreas)) {
    throw new Error("requiredPlatformCoverageAreas must be an array")
  }
  const areas = []
  for (const area of requiredPlatformCoverageAreas) {
    if (!nonEmptyString(area)) {
      throw new Error("requiredPlatformCoverageAreas has invalid area")
    }
    for (const value of area.split(",")) {
      const normalized = value.trim()
      if (normalized) areas.push(normalized)
    }
  }
  return [...new Set(areas)].sort()
}

export function normalizeRequiredPresets(presets) {
  if (!Array.isArray(presets)) {
    throw new Error("presets must be an array")
  }
  const names = []
  for (const preset of presets) {
    if (!nonEmptyString(preset)) {
      throw new Error("presets has invalid preset")
    }
    for (const value of preset.split(",")) {
      const normalized = value.trim()
      if (normalized) names.push(normalized)
    }
  }
  const normalizedNames = [...new Set(names)].sort()
  for (const preset of normalizedNames) {
    if (!Object.prototype.hasOwnProperty.call(DRILL_VALIDATION_GATE_PRESETS, preset)) {
      throw new Error(`unknown validation gate preset: ${preset}`)
    }
  }
  return normalizedNames
}

export function isKnownDrillValidationGatePreset(preset) {
  return typeof preset === "string"
    && Object.prototype.hasOwnProperty.call(DRILL_VALIDATION_GATE_PRESETS, preset)
}

export function isKnownDrillArtifactValidationPreset(preset) {
  return typeof preset === "string"
    && DRILL_ARTIFACT_VALIDATION_PRESETS.includes(preset)
}

export function normalizeRequiredFailureClassifications(requiredFailureClassifications) {
  if (!Array.isArray(requiredFailureClassifications)) {
    throw new Error("requiredFailureClassifications must be an array")
  }
  const classifications = []
  for (const classification of requiredFailureClassifications) {
    if (!nonEmptyString(classification)) {
      throw new Error("requiredFailureClassifications has invalid classification")
    }
    for (const value of classification.split(",")) {
      const normalized = value.trim()
      if (normalized) classifications.push(normalized)
    }
  }
  const normalizedClassifications = [...new Set(classifications)].sort()
  for (const classification of normalizedClassifications) {
    if (!isKnownDrillFailureClassification(classification)) {
      throw new Error(`unknown required failure classification: ${classification}`)
    }
  }
  return normalizedClassifications
}

export function normalizeRequiredMatrices(requiredMatrices) {
  if (!Array.isArray(requiredMatrices)) {
    throw new Error("requiredMatrices must be an array")
  }
  const matrices = []
  for (const matrix of requiredMatrices) {
    if (!nonEmptyString(matrix)) {
      throw new Error("requiredMatrices has invalid matrix")
    }
    for (const value of matrix.split(",")) {
      const normalized = value.trim()
      if (normalized) matrices.push(normalized)
    }
  }
  return [...new Set(matrices)].sort()
}

export function normalizeRequiredMatrixClassifications(requiredMatrixClassifications) {
  if (!Array.isArray(requiredMatrixClassifications)) {
    throw new Error("requiredMatrixClassifications must be an array")
  }
  const classifications = []
  for (const classification of requiredMatrixClassifications) {
    if (!nonEmptyString(classification)) {
      throw new Error("requiredMatrixClassifications has invalid classification")
    }
    for (const value of classification.split(",")) {
      const normalized = value.trim()
      if (normalized) classifications.push(normalized)
    }
  }
  const normalizedClassifications = [...new Set(classifications)].sort()
  for (const classification of normalizedClassifications) {
    if (!isKnownDrillFailureClassification(classification)) {
      throw new Error(`unknown required matrix classification: ${classification}`)
    }
  }
  return normalizedClassifications
}

export function normalizeRequiredMatrixRuntimeSignals(requiredMatrixRuntimeSignals) {
  if (!Array.isArray(requiredMatrixRuntimeSignals)) {
    throw new Error("requiredMatrixRuntimeSignals must be an array")
  }
  const signals = []
  for (const signal of requiredMatrixRuntimeSignals) {
    if (!nonEmptyString(signal)) {
      throw new Error("requiredMatrixRuntimeSignals has invalid signal")
    }
    for (const value of signal.split(",")) {
      const normalized = value.trim()
      if (normalized) signals.push(normalized)
    }
  }
  const normalizedSignals = [...new Set(signals)].sort()
  for (const signal of normalizedSignals) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`unknown required matrix runtime signal: ${signal}`)
    }
  }
  return normalizedSignals
}

export function normalizeRequiredDeploymentPresets(requiredDeploymentPresets) {
  if (!Array.isArray(requiredDeploymentPresets)) {
    throw new Error("requiredDeploymentPresets must be an array")
  }
  const presets = []
  for (const preset of requiredDeploymentPresets) {
    if (!nonEmptyString(preset)) {
      throw new Error("requiredDeploymentPresets has invalid preset")
    }
    for (const value of preset.split(",")) {
      const normalized = value.trim()
      if (normalized) presets.push(normalized)
    }
  }
  const normalizedPresets = [...new Set(presets)].sort()
  for (const preset of normalizedPresets) {
    if (!DRILL_DEPLOYMENT_PRESETS.includes(preset)) {
      throw new Error(`unknown required deployment preset: ${preset}`)
    }
  }
  return normalizedPresets
}

export function normalizeRequiredProviders(requiredProviders) {
  const providers = normalizeCommaSeparatedStrings(requiredProviders, {
    fieldName: "requiredProviders",
    itemName: "provider",
  })
  for (const provider of providers) {
    if (!isKnownDrillProvider(provider)) {
      throw new Error(`unknown required provider: ${provider}`)
    }
  }
  return providers
}

export function normalizeRequiredScenarios(requiredScenarios) {
  if (!Array.isArray(requiredScenarios)) {
    throw new Error("requiredScenarios must be an array")
  }
  const scenarios = []
  for (const scenario of requiredScenarios) {
    if (!nonEmptyString(scenario)) {
      throw new Error("requiredScenarios has invalid scenario")
    }
    for (const value of scenario.split(",")) {
      const normalized = value.trim()
      if (normalized) scenarios.push(normalized)
    }
  }
  return [...new Set(scenarios)].sort()
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function normalizeCommaSeparatedStrings(values, { fieldName, itemName }) {
  if (!Array.isArray(values)) {
    throw new Error(`${fieldName} must be an array`)
  }
  const normalizedValues = []
  for (const value of values) {
    if (!nonEmptyString(value)) {
      throw new Error(`${fieldName} has invalid ${itemName}`)
    }
    for (const part of value.split(",")) {
      const normalized = part.trim()
      if (normalized) normalizedValues.push(normalized)
    }
  }
  return [...new Set(normalizedValues)].sort()
}

function normalizeGeneratedEvidencePathRequirements(values, { fieldName, itemName }) {
  const paths = normalizeCommaSeparatedStrings(values, { fieldName, itemName })
  for (const [index, valuePath] of paths.entries()) {
    validateDrillGeneratedEvidencePath(valuePath, `${fieldName}[${index}]`)
  }
  return paths
}

function normalizeDiagnosticTextRequirements(values, { fieldName, itemName }) {
  const texts = normalizeCommaSeparatedStrings(values, { fieldName, itemName })
  for (const [index, text] of texts.entries()) {
    if (redactDrillSecretText(text) !== text) {
      throw new Error(`${fieldName}[${index}] includes secret-looking diagnostic text`)
    }
  }
  return texts
}

function validateExitCriterionStatuses(statuses, fieldName) {
  for (const status of statuses) {
    if (!["satisfied", "failed", "skipped", "dry-run"].includes(status)) {
      throw new Error(`unknown required artifact exit criterion status in ${fieldName}: ${status}`)
    }
  }
}
