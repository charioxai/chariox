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
  ])
  assert.deepEqual(fixtures.map((fixture) => fixture.report.matrix), [
    "native-provider-tui-matrix",
    "remote-agent-runtime-matrix",
    "slice-runtime-matrix",
  ])

  const scenarios = fixtures.flatMap((fixture) => fixture.report.scenarios)
  assert.deepEqual([...new Set(scenarios.map((scenario) => scenario.id))].sort(), [
    "agent-reuse",
    "collab-remote-agent",
    "hetzner-collab-remote-agent",
    "hetzner-single-user-remote-agent",
    "hosted-collab-remote-agent",
    "hosted-single-user-remote-agent",
    "lease-reconnect",
    "local-native-tui",
    "permission-visibility",
    "provider-run-binding",
    "remote-native-tui",
    "remote-prompt-dispatch",
    "session-start",
    "single-user-remote-agent",
    "slice-native-tui",
    "transcript-parity",
    "ui-projection",
  ])
  assert.deepEqual([...new Set(scenarios.flatMap((scenario) => scenario.runtimeSignals))].sort(), [
    "agent-lifecycle",
    "client-projection-health",
    "lease-health",
    "permission-interaction",
    "provider-run-lifecycle",
    "runtime-projection-health",
    "session-authority",
  ])
  assert.deepEqual([...new Set(scenarios.map((scenario) => scenario.classification))].sort(), [
    "kernel-authority",
    "provider-auth",
    "provider-error",
    "relay-runtime",
    "ui-client-projection",
    "worker-execution",
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
