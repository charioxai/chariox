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

export function workspaceLiveSyncScenarioProvider(id) {
  return id.includes("opencode") ? "opencode" : "codex"
}

export function workspaceLiveSyncScenarioMode(id) {
  if (id.includes("off")) return "off"
  if (id.includes("managed")) return "managed"
  if (id.includes("tracked")) return "tracked"
  if (id.includes("permission")) return "permission"
  return "unknown"
}

export function workspaceLiveSyncScenarioDeployment(id) {
  if (id.startsWith("hetzner-")) return "hetzner"
  if (id.startsWith("remote-")) return "same-host-remote"
  return "local"
}

export function workspaceLiveSyncScenarioRequires(id) {
  const requires = []
  if (id.startsWith("remote-") || id.startsWith("hetzner-")) requires.push("remote")
  if (id.startsWith("hetzner-")) requires.push("hetzner")
  if (workspaceLiveSyncScenarioProvider(id) === "opencode") requires.push("opencode")
  return requires
}

export function workspaceLiveSyncRequiredProviders() {
  return [...new Set(workspaceLiveSyncRequiredScenarioIds().map(workspaceLiveSyncScenarioProvider))].sort()
}

export function workspaceLiveSyncRequiredDeployments() {
  return [...new Set(workspaceLiveSyncRequiredScenarioIds().map(workspaceLiveSyncScenarioDeployment))].sort()
}

export function workspaceLiveSyncRequiredModes() {
  return [...new Set(workspaceLiveSyncRequiredScenarioIds().map(workspaceLiveSyncScenarioMode))].sort()
}

export function workspaceLiveSyncRequiredScenarioDescriptors() {
  return workspaceLiveSyncRequiredScenarioIds().map((id) => ({
    id,
    classification: workspaceLiveSyncScenarioClassification(id),
    deployment: workspaceLiveSyncScenarioDeployment(id),
    mode: workspaceLiveSyncScenarioMode(id),
    provider: workspaceLiveSyncScenarioProvider(id),
    requires: workspaceLiveSyncScenarioRequires(id),
    runtimeSignals: workspaceLiveSyncScenarioRuntimeSignals(id),
  }))
}
