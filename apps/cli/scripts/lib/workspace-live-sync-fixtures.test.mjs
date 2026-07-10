import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { fileExists } from "./workspace-live-sync-drill-runtime.mjs"

import {
  workspaceLiveSyncRequiredDeployments,
  workspaceLiveSyncRequiredModes,
  workspaceLiveSyncRequiredProviders,
  workspaceLiveSyncRequiredScenarioDescriptors,
  workspaceLiveSyncRequiredScenarioIds,
  workspaceLiveSyncScenarioClassification,
  workspaceLiveSyncScenarioDeployment,
  workspaceLiveSyncScenarioMode,
  workspaceLiveSyncScenarioProvider,
  workspaceLiveSyncScenarioRequires,
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

test("workspace live sync fixture derives scenario routing metadata", () => {
  assert.equal(workspaceLiveSyncScenarioProvider("local-tracked-opencode"), "opencode")
  assert.equal(workspaceLiveSyncScenarioProvider("remote-tracked-codex"), "codex")
  assert.equal(workspaceLiveSyncScenarioMode("local-off-codex"), "off")
  assert.equal(workspaceLiveSyncScenarioMode("local-managed-opencode"), "managed")
  assert.equal(workspaceLiveSyncScenarioMode("remote-tracked-codex"), "tracked")
  assert.equal(workspaceLiveSyncScenarioMode("hetzner-permission-opencode"), "permission")
  assert.equal(workspaceLiveSyncScenarioDeployment("local-tracked-codex"), "local")
  assert.equal(workspaceLiveSyncScenarioDeployment("remote-tracked-codex"), "same-host-remote")
  assert.equal(workspaceLiveSyncScenarioDeployment("hetzner-tracked-codex"), "hetzner")
  assert.deepEqual(workspaceLiveSyncScenarioRequires("local-managed-codex"), [])
  assert.deepEqual(workspaceLiveSyncScenarioRequires("local-managed-opencode"), ["opencode"])
  assert.deepEqual(workspaceLiveSyncScenarioRequires("remote-tracked-opencode"), ["remote", "opencode"])
  assert.deepEqual(workspaceLiveSyncScenarioRequires("hetzner-tracked-opencode"), ["remote", "hetzner", "opencode"])
  assert.deepEqual(workspaceLiveSyncRequiredProviders(), ["codex", "opencode"])
  assert.deepEqual(workspaceLiveSyncRequiredDeployments(), ["hetzner", "local", "same-host-remote"])
  assert.deepEqual(workspaceLiveSyncRequiredModes(), ["managed", "off", "permission", "tracked"])
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
    deployment: "hetzner",
    mode: "permission",
    provider: "codex",
    requires: ["remote", "hetzner"],
    runtimeSignals: ["session-authority", "workspace-live-sync-state"],
  })
  assert.deepEqual(workspaceLiveSyncRequiredScenarioDescriptors().at(-1), {
    id: "remote-tracked-restart-codex",
    classification: "relay-target-freshness",
    deployment: "same-host-remote",
    mode: "tracked",
    provider: "codex",
    requires: ["remote"],
    runtimeSignals: ["relay-target-freshness", "session-authority", "workspace-live-sync-state"],
  })
})

test("workspace live sync file existence probes distinguish present and missing paths", async (t) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "workspace-live-sync-file-exists-"))
  t.after(async () => await rm(root, { recursive: true, force: true }))
  const present = path.join(root, "present.txt")
  await writeFile(present, "present\n", "utf8")

  assert.equal(await fileExists(present), true)
  assert.equal(await fileExists(path.join(root, "missing.txt")), false)
})
