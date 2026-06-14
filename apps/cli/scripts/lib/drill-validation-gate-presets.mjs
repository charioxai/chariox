import { DRILL_DEPLOYMENT_PRESETS } from "./drill-environment-presets.mjs"
import { isKnownDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import { isKnownDrillRuntimeSignal } from "./drill-runtime-signals.mjs"

export const DRILL_VALIDATION_GATE_PRESETS = Object.freeze({
  "distributed-runtime": Object.freeze({
    description: "End-to-end distributed runtime authority evidence across native TUI, remote agents, home extensions, slices, and Workspace Live Sync.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze([
      "agent-lifecycle",
      "client-projection-health",
      "home-extension-manifest-sync",
      "lease-health",
      "permission-interaction",
      "provider-run-lifecycle",
      "relay-target-freshness",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
      "workspace-live-sync-state",
    ]),
    requiredFailureClassifications: Object.freeze([
      "docker-runtime",
      "kernel-authority",
      "provider-auth",
      "provider-error",
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
    requiredDeploymentPresets: Object.freeze(["hetzner", "hosted-cloud", "local", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze([
      "agent-reuse",
      "collab-remote-agent",
      "hetzner-collab",
      "hetzner-single",
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
    requiredRuntimeSignals: Object.freeze(["client-projection-health", "permission-interaction", "provider-run-lifecycle", "session-authority"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"]),
    requiredMatrices: Object.freeze(["native-provider-tui-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "ui-client-projection", "worker-execution"]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "local", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze(["local-native-tui", "permission-visibility", "remote-native-tui", "slice-native-tui", "transcript-parity"]),
  }),
  "remote-agent-runtime": Object.freeze({
    description: "Leased remote-agent lifecycle, worker provider-run binding, relay freshness, and collab projection evidence.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["agent-lifecycle", "client-projection-health", "lease-health", "provider-run-lifecycle", "relay-target-freshness", "session-authority"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-host-capacity", "remote-worker-version", "ui-client-projection", "worker-execution"]),
    requiredMatrices: Object.freeze(["remote-agent-runtime-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "ui-client-projection", "worker-execution"]),
    requiredDeploymentPresets: Object.freeze(["hetzner", "hosted-cloud", "same-host-remote", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze(["collab-remote-agent", "lease-reconnect", "provider-run-binding", "remote-prompt-dispatch", "single-user-remote-agent"]),
  }),
  "workspace-live-sync": Object.freeze({
    description: "Workspace Live Sync local/remote matrix evidence and distributed sync diagnostics.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"]),
    requiredMatrices: Object.freeze(["workspace-live-sync-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"]),
  }),
  "remote-home-extension": Object.freeze({
    description: "Home-owned extension execution evidence for remote agents and collab authority checks.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "worker-execution"]),
    requiredMatrices: Object.freeze(["remote-home-extension-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "remote-extension-sync", "worker-execution"]),
  }),
  "slice-runtime": Object.freeze({
    description: "Slice lifecycle, provider-auth isolation, worker discovery, and UI projection evidence.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredRuntimeSignals: Object.freeze(["agent-lifecycle", "client-projection-health", "provider-run-lifecycle", "session-authority", "slice-auth-state", "slice-runtime-state"]),
    requiredFailureClassifications: Object.freeze(["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"]),
    requiredMatrices: Object.freeze(["slice-runtime-matrix"]),
    requiredMatrixClassifications: Object.freeze(["docker-runtime", "kernel-authority", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution"]),
    requiredDeploymentPresets: Object.freeze(["hosted-cloud", "local", "self-hosted-relay"]),
    requiredProviders: Object.freeze(["claude", "codex", "opencode"]),
    requiredScenarios: Object.freeze(["agent-reuse", "provider-auth", "session-start", "slice-lifecycle", "ui-projection"]),
  }),
})

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
      requiredRuntimeSignals: [...(preset.requiredRuntimeSignals ?? [])],
      requiredFailureClassifications: [...(preset.requiredFailureClassifications ?? [])],
      requiredMatrices: [...(preset.requiredMatrices ?? [])],
      requiredMatrixClassifications: [...(preset.requiredMatrixClassifications ?? [])],
      requiredDeploymentPresets: [...(preset.requiredDeploymentPresets ?? [])],
      requiredProviders: [...(preset.requiredProviders ?? [])],
      requiredScenarios: [...(preset.requiredScenarios ?? [])],
    }
  })
}

export function expandValidationGatePresetRequirements({
  presets,
  requiredPlatformCoverageAreas,
  requiredRuntimeSignals = [],
  requiredFailureClassifications,
  requiredMatrices,
  requiredMatrixClassifications,
  requiredDeploymentPresets,
  requiredProviders,
  requiredScenarios,
}) {
  const expanded = {
    requiredPlatformCoverageAreas: [...requiredPlatformCoverageAreas],
    requiredRuntimeSignals: [...requiredRuntimeSignals],
    requiredFailureClassifications: [...requiredFailureClassifications],
    requiredMatrices: [...requiredMatrices],
    requiredMatrixClassifications: [...requiredMatrixClassifications],
    requiredDeploymentPresets: [...requiredDeploymentPresets],
    requiredProviders: [...requiredProviders],
    requiredScenarios: [...requiredScenarios],
  }
  for (const presetName of presets) {
    const preset = DRILL_VALIDATION_GATE_PRESETS[presetName]
    expanded.requiredPlatformCoverageAreas.push(...(preset.requiredPlatformCoverageAreas ?? []))
    expanded.requiredRuntimeSignals.push(...(preset.requiredRuntimeSignals ?? []))
    expanded.requiredFailureClassifications.push(...(preset.requiredFailureClassifications ?? []))
    expanded.requiredMatrices.push(...(preset.requiredMatrices ?? []))
    expanded.requiredMatrixClassifications.push(...(preset.requiredMatrixClassifications ?? []))
    expanded.requiredDeploymentPresets.push(...(preset.requiredDeploymentPresets ?? []))
    expanded.requiredProviders.push(...(preset.requiredProviders ?? []))
    expanded.requiredScenarios.push(...(preset.requiredScenarios ?? []))
  }
  return expanded
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
  if (!Array.isArray(requiredProviders)) {
    throw new Error("requiredProviders must be an array")
  }
  const providers = []
  for (const provider of requiredProviders) {
    if (!nonEmptyString(provider)) {
      throw new Error("requiredProviders has invalid provider")
    }
    for (const value of provider.split(",")) {
      const normalized = value.trim()
      if (normalized) providers.push(normalized)
    }
  }
  return [...new Set(providers)].sort()
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
