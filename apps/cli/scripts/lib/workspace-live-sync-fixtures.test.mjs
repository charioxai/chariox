import assert from "node:assert/strict"
import test from "node:test"

import {
  workspaceLiveSyncRequiredScenarioDescriptors,
  workspaceLiveSyncRequiredScenarioIds,
  workspaceLiveSyncScenarioClassification,
  workspaceLiveSyncScenarioRuntimeSignals,
} from "./workspace-live-sync-fixtures.mjs"

test("workspace live sync fixture lists required validation scenarios", () => {
  assert.deepEqual(workspaceLiveSyncRequiredScenarioIds(), [
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
})

test("workspace live sync fixture derives scenario evidence metadata", () => {
  assert.equal(workspaceLiveSyncScenarioClassification("local-off-codex"), null)
  assert.equal(workspaceLiveSyncScenarioClassification("local-managed-codex"), "workspace-live-sync-conflict")
  assert.equal(workspaceLiveSyncScenarioClassification("remote-tracked-opencode"), "workspace-live-sync-conflict")
  assert.equal(workspaceLiveSyncScenarioClassification("local-permission-codex"), "kernel-authority")
  assert.equal(workspaceLiveSyncScenarioClassification("remote-tracked-restart-codex"), "relay-target-freshness")

  assert.deepEqual(workspaceLiveSyncScenarioRuntimeSignals("local-off-codex"), ["session-authority"])
  assert.deepEqual(workspaceLiveSyncScenarioRuntimeSignals("local-managed-codex"), ["session-authority", "workspace-live-sync-state"])
  assert.deepEqual(workspaceLiveSyncScenarioRuntimeSignals("remote-tracked-restart-codex"), [
    "relay-target-freshness",
    "session-authority",
    "workspace-live-sync-state",
  ])
})

test("workspace live sync descriptors combine ids and evidence metadata", () => {
  assert.deepEqual(workspaceLiveSyncRequiredScenarioDescriptors()[0], {
    id: "hetzner-permission-codex",
    classification: "kernel-authority",
    runtimeSignals: ["session-authority", "workspace-live-sync-state"],
  })
  assert.deepEqual(workspaceLiveSyncRequiredScenarioDescriptors().at(-1), {
    id: "remote-tracked-restart-codex",
    classification: "relay-target-freshness",
    runtimeSignals: ["relay-target-freshness", "session-authority", "workspace-live-sync-state"],
  })
})
