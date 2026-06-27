import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_RUNTIME_SIGNAL_IDS,
  DRILL_RUNTIME_SIGNALS_SCHEMA,
  drillRuntimeSignalOwnerCounts,
  drillRuntimeSignalNextAction,
  drillRuntimeSignalOwner,
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalsManifest,
  isKnownDrillRuntimeSignalOwner,
  isKnownDrillRuntimeSignal,
  validateDrillRuntimeSignal,
  validateDrillRuntimeSignalOwner,
  validateDrillRuntimeSignals,
  validateDrillRuntimeSignalsManifest,
} from "./drill-runtime-signals.mjs"

test("defines stable distributed runtime signal ids", () => {
  assert.deepEqual(DRILL_RUNTIME_SIGNAL_IDS, [
    "agent-lifecycle",
    "client-projection-health",
    "home-extension-manifest-sync",
    "lease-health",
    "permission-interaction",
    "provider-run-lifecycle",
    "relay-target-freshness",
    "runtime-projection-health",
    "runtime-transition-audit",
    "session-authority",
    "slice-auth-state",
    "slice-runtime-state",
    "workspace-live-sync-state",
  ])
})

test("writes and validates runtime signal manifest", () => {
  const manifest = drillRuntimeSignalsManifest()

  assert.equal(manifest.schema, DRILL_RUNTIME_SIGNALS_SCHEMA)
  assert.deepEqual(manifest.signals.map((signal) => signal.id), DRILL_RUNTIME_SIGNAL_IDS)
  assert.equal(drillRuntimeSignalOwner("session-authority"), "kernel-authority")
  assert.equal(drillRuntimeSignalOwner("runtime-transition-audit"), "kernel-authority")
  assert.equal(drillRuntimeSignalOwner("slice-auth-state"), "provider-account")
  assert.equal(
    drillRuntimeSignalNextAction("lease-health"),
    "run matrix scenarios that prove lease-health owned by kernel-authority: Remote leased-agent session, worker, reconnect, and provider-run binding health.",
  )
  assert.equal(
    drillRuntimeSignalNextAction("slice-auth-state", { target: "platform-bundle" }),
    "add runtime-signal contract coverage for slice-auth-state owned by provider-account to the drill platform bundle",
  )
  assert.equal(
    drillRuntimeSignalNextAction("relay-target-freshness", { target: "artifact-index" }),
    "include drill artifact indexes proving relay-target-freshness owned by runtime-network: Relay target heartbeat freshness, selected kernel identity, and stale-target rejection.",
  )
  assert.deepEqual(drillRuntimeSignalOwnersFor(["slice-auth-state", "session-authority", "lease-health"]), [
    "kernel-authority",
    "provider-account",
  ])
  assert.deepEqual(drillRuntimeSignalOwnerCounts({
    "lease-health": 2,
    "provider-run-lifecycle": 1,
    "relay-target-freshness": 1,
    "runtime-transition-audit": 2,
    "runtime-projection-health": 3,
  }), {
    "kernel-authority": 7,
    "provider-runtime": 1,
    "runtime-network": 1,
  })
  assert.equal(isKnownDrillRuntimeSignal("workspace-live-sync-state"), true)
  assert.equal(isKnownDrillRuntimeSignal("workspace-live-synch-state"), false)
  assert.equal(isKnownDrillRuntimeSignalOwner("kernel-authority"), true)
  assert.equal(isKnownDrillRuntimeSignalOwner("kernel-authoritiy"), false)
  assert.doesNotThrow(() => validateDrillRuntimeSignal("workspace-live-sync-state", "report.runtimeSignals[0]"))
  assert.doesNotThrow(() => validateDrillRuntimeSignalOwner("kernel-authority", "report.runtimeSignalOwners[0]"))
  assert.throws(
    () => validateDrillRuntimeSignal("workspace-live-synch-state", "report.runtimeSignals[0]"),
    /report\.runtimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillRuntimeSignalOwner("kernel-authoritiy", "report.runtimeSignalOwners[0]"),
    /report\.runtimeSignalOwners\[0\] has unknown runtime signal owner "kernel-authoritiy"/,
  )
  assert.throws(
    () => validateDrillRuntimeSignal("workspace-live-synch-state", "manifest.signals[0]", { label: "id" }),
    /manifest\.signals\[0\] has unknown id "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillRuntimeSignal("workspace-live-synch-state", "--require-runtime-signal", {
      message: (signal) => `--require-runtime-signal has unknown runtime signal: ${signal}`,
    }),
    /--require-runtime-signal has unknown runtime signal: workspace-live-synch-state/,
  )
  assert.throws(
    () => validateDrillRuntimeSignalOwner("kernel-authoritiy", "--require-runtime-signal-owner", {
      message: (owner) => `--require-runtime-signal-owner has unknown runtime signal owner: ${owner}`,
    }),
    /--require-runtime-signal-owner has unknown runtime signal owner: kernel-authoritiy/,
  )
  assert.doesNotThrow(() => validateDrillRuntimeSignalsManifest(manifest))
})

test("rejects unknown drill runtime signals in shared helpers", () => {
  assert.throws(
    () => drillRuntimeSignalOwner("workspace-live-synch-state"),
    /unknown drill runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => drillRuntimeSignalNextAction("workspace-live-synch-state"),
    /unknown drill runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => drillRuntimeSignalOwnersFor(["workspace-live-sync-state", "workspace-live-synch-state"]),
    /drill runtime signals\[1\] has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => drillRuntimeSignalOwnerCounts({ "workspace-live-synch-state": 1 }),
    /unknown drill runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => drillRuntimeSignalOwnerCounts({ "session-authority": 0.5 }),
    /drill runtime signal "session-authority" has invalid count/,
  )
  assert.throws(
    () => drillRuntimeSignalOwnerCounts({ "session-authority": -1 }),
    /drill runtime signal "session-authority" has invalid count/,
  )
  assert.throws(
    () => validateDrillRuntimeSignals(["session-authority", "runtime-projector-health"], "report.runtimeSignals"),
    /report\.runtimeSignals\[1\] has unknown runtime signal "runtime-projector-health"/,
  )
})

test("rejects runtime signal manifest drift", () => {
  const manifest = drillRuntimeSignalsManifest()

  assert.throws(
    () => validateDrillRuntimeSignalsManifest({
      ...manifest,
      signals: manifest.signals.filter((signal) => signal.id !== "lease-health"),
    }),
    /does not match required runtime signals/,
  )
  assert.throws(
    () => validateDrillRuntimeSignalsManifest({
      ...manifest,
      signals: manifest.signals.map((signal) => signal.id === "session-authority"
        ? { ...signal, owner: "ui-client" }
        : signal),
    }),
    /has invalid owner/,
  )
  assert.throws(
    () => validateDrillRuntimeSignalsManifest({
      ...manifest,
      signals: [...manifest.signals, manifest.signals[0]],
    }),
    /duplicate signal/,
  )
})
