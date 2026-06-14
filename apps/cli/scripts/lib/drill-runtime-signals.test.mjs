import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_RUNTIME_SIGNAL_IDS,
  DRILL_RUNTIME_SIGNALS_SCHEMA,
  drillRuntimeSignalOwnerCounts,
  drillRuntimeSignalOwner,
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalsManifest,
  isKnownDrillRuntimeSignal,
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
  assert.equal(drillRuntimeSignalOwner("slice-auth-state"), "provider-account")
  assert.deepEqual(drillRuntimeSignalOwnersFor(["slice-auth-state", "session-authority", "lease-health"]), [
    "kernel-authority",
    "provider-account",
  ])
  assert.deepEqual(drillRuntimeSignalOwnerCounts({
    "lease-health": 2,
    "provider-run-lifecycle": 1,
    "relay-target-freshness": 1,
  }), {
    "kernel-authority": 2,
    "provider-runtime": 1,
    "runtime-network": 1,
  })
  assert.equal(isKnownDrillRuntimeSignal("workspace-live-sync-state"), true)
  assert.equal(isKnownDrillRuntimeSignal("workspace-live-synch-state"), false)
  assert.doesNotThrow(() => validateDrillRuntimeSignalsManifest(manifest))
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
