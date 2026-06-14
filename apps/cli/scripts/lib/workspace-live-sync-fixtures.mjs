export const WORKSPACE_LIVE_SYNC_REQUIRED_SCENARIO_IDS = Object.freeze([
  "hetzner-permission-codex",
  "hetzner-permission-opencode",
  "hetzner-tracked-codex",
  "hetzner-tracked-opencode",
  "local-managed-codex",
  "local-managed-opencode",
  "local-off-codex",
  "local-permission-codex",
  "local-permission-opencode",
  "local-tracked-codex",
  "local-tracked-opencode",
  "remote-managed-codex",
  "remote-managed-opencode",
  "remote-permission-codex",
  "remote-permission-opencode",
  "remote-tracked-codex",
  "remote-tracked-opencode",
  "remote-tracked-restart-codex",
])

export function workspaceLiveSyncRequiredScenarioIds() {
  return [...WORKSPACE_LIVE_SYNC_REQUIRED_SCENARIO_IDS]
}

export function workspaceLiveSyncScenarioClassification(id) {
  if (id.includes("permission")) return "kernel-authority"
  if (id.includes("restart")) return "relay-target-freshness"
  if (id.includes("managed") || id.includes("tracked")) return "workspace-live-sync-conflict"
  return null
}

export function workspaceLiveSyncScenarioRuntimeSignals(id) {
  const signals = ["session-authority"]
  if (id.includes("restart")) signals.push("relay-target-freshness")
  if (id.includes("permission") || id.includes("managed") || id.includes("tracked")) {
    signals.push("workspace-live-sync-state")
  }
  return [...new Set(signals)].sort()
}

export function workspaceLiveSyncRequiredScenarioDescriptors() {
  return workspaceLiveSyncRequiredScenarioIds().map((id) => ({
    id,
    classification: workspaceLiveSyncScenarioClassification(id),
    runtimeSignals: workspaceLiveSyncScenarioRuntimeSignals(id),
  }))
}
