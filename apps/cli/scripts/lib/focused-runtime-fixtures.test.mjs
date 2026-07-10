import assert from "node:assert/strict"
import test from "node:test"

import {
  distributedStateHealthPartialMatrixReport,
  runtimeAuthorityMatrixReportFixtures,
} from "./focused-runtime-fixtures.mjs"

test("runtime authority fixtures cover focused matrix contracts", () => {
  const fixtures = runtimeAuthorityMatrixReportFixtures()
  assert.deepEqual(fixtures.map((fixture) => fixture.fileName), [
    "native-provider-tui.json",
    "remote-agent-runtime.json",
    "slice-runtime.json",
    "browser-terminal-resilience.json",
    "cloud-slice-runtime.json",
    "remote-home-extension.json",
    "runtime-resilience-chaos.json",
    "workspace-live-sync.json",
  ])
  assert.deepEqual(fixtures.map((fixture) => fixture.report.matrix), [
    "native-provider-tui-matrix",
    "remote-agent-runtime-matrix",
    "slice-runtime-matrix",
    "browser-terminal-resilience-matrix",
    "cloud-slice-runtime-matrix",
    "remote-home-extension-matrix",
    "runtime-resilience-chaos-matrix",
    "workspace-live-sync-matrix",
  ])

  const scenarios = fixtures.flatMap((fixture) => fixture.report.scenarios)
  assert.deepEqual([...new Set(scenarios.map((scenario) => scenario.id))].sort(), [
    "agent-reuse",
    "collab-remote-agent",
    "hetzner-collab",
    "hetzner-collab-remote-agent",
    "hetzner-single",
    "hetzner-single-user-remote-agent",
    "hosted-collab-remote-agent",
    "hosted-single-user-remote-agent",
    "lease-reconnect",
    "local-browser-relay-kernel-reconnect",
    "local-collab",
    "local-kernel-restart-durable-state",
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
    "remote-tracked-restart-codex",
    "session-start",
    "single-user-remote-agent",
    "slice-lifecycle",
    "slice-native-tui",
    "transcript-parity",
    "ui-projection",
  ])
  assert.deepEqual([...new Set(scenarios.flatMap((scenario) => scenario.runtimeSignals))].sort(), [
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
  ])
  assert.deepEqual([...new Set(scenarios.map((scenario) => scenario.classification))].sort(), [
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
  ])
})

test("distributed state health partial fixture leaves owner-routed gaps", () => {
  const report = distributedStateHealthPartialMatrixReport()

  assert.equal(report.matrix, "remote-agent-runtime-matrix")
  assert.equal(report.metadata.deploymentPresets, "local")
  assert.equal(report.metadata.providers, "codex")
  assert.deepEqual(report.scenarios.map((scenario) => scenario.id), ["lease-reconnect"])
  assert.deepEqual(report.scenarios[0].runtimeSignals, ["lease-health", "provider-run-lifecycle"])
  assert.equal(report.scenarios[0].classification, "kernel-authority")
})
