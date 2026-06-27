import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS,
  DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA,
  drillRuntimeAuthorityInvariantOwner,
  drillRuntimeAuthorityInvariantSignals,
  drillRuntimeAuthorityManifest,
  isKnownDrillRuntimeAuthorityInvariant,
  validateDrillRuntimeAuthorityInvariant,
  validateDrillRuntimeAuthorityManifest,
} from "./drill-runtime-authority-invariants.mjs"

test("defines stable runtime authority invariant ids", () => {
  assert.deepEqual(DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS, [
    "client-render-request",
    "home-session-authority",
    "primary-provider-run-per-agent",
    "projected-state-diagnostics",
    "relay-cloud-transport-only",
    "session-scoped-agent-identity",
    "shared-runtime-primitives",
    "worker-execution-authority",
  ])
})

test("writes and validates runtime authority manifest", () => {
  const manifest = drillRuntimeAuthorityManifest()

  assert.equal(manifest.schema, DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA)
  assert.deepEqual(manifest.invariants.map((invariant) => invariant.id), DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS)
  assert.equal(drillRuntimeAuthorityInvariantOwner("home-session-authority"), "kernel-authority")
  assert.equal(drillRuntimeAuthorityInvariantOwner("worker-execution-authority"), "worker-kernel")
  assert.deepEqual(drillRuntimeAuthorityInvariantSignals("relay-cloud-transport-only"), [
    "relay-target-freshness",
    "session-authority",
  ])
  assert.deepEqual(drillRuntimeAuthorityInvariantSignals("primary-provider-run-per-agent"), [
    "agent-lifecycle",
    "lease-health",
    "provider-run-lifecycle",
    "session-authority",
  ])
  assert.deepEqual(drillRuntimeAuthorityInvariantSignals("session-scoped-agent-identity"), [
    "agent-lifecycle",
    "client-projection-health",
    "lease-health",
    "runtime-projection-health",
    "session-authority",
  ])
  assert.equal(isKnownDrillRuntimeAuthorityInvariant("shared-runtime-primitives"), true)
  assert.equal(isKnownDrillRuntimeAuthorityInvariant("shared-runtime-primitive"), false)
  assert.doesNotThrow(() => validateDrillRuntimeAuthorityInvariant("client-render-request", "manifest.invariants[0]"))
  assert.throws(
    () => validateDrillRuntimeAuthorityInvariant("client-renders-state", "manifest.invariants[0]"),
    /manifest\.invariants\[0\] has unknown runtime authority invariant "client-renders-state"/,
  )
  assert.doesNotThrow(() => validateDrillRuntimeAuthorityManifest(manifest))
})

test("rejects runtime authority manifest drift", () => {
  const manifest = drillRuntimeAuthorityManifest()

  assert.throws(
    () => validateDrillRuntimeAuthorityManifest({
      ...manifest,
      invariants: manifest.invariants.filter((invariant) => invariant.id !== "home-session-authority"),
    }),
    /does not match required runtime authority invariants/,
  )
  assert.throws(
    () => validateDrillRuntimeAuthorityManifest({
      ...manifest,
      invariants: manifest.invariants.map((invariant) => invariant.id === "worker-execution-authority"
        ? { ...invariant, owner: "kernel-authority" }
        : invariant),
    }),
    /has invalid owner/,
  )
  assert.throws(
    () => validateDrillRuntimeAuthorityManifest({
      ...manifest,
      invariants: manifest.invariants.map((invariant) => invariant.id === "relay-cloud-transport-only"
        ? { ...invariant, requiredRuntimeSignals: ["session-authority"] }
        : invariant),
    }),
    /requiredRuntimeSignals do not match required runtime signals/,
  )
  assert.throws(
    () => validateDrillRuntimeAuthorityManifest({
      ...manifest,
      invariants: manifest.invariants.map((invariant) => invariant.id === "client-render-request"
        ? { ...invariant, requiredRuntimeSignals: ["session-authority", "client-projection-health", "invented-signal"] }
        : invariant),
    }),
    /has unknown runtime signal "invented-signal"/,
  )
  assert.throws(
    () => validateDrillRuntimeAuthorityManifest({
      ...manifest,
      invariants: [...manifest.invariants, manifest.invariants[0]],
    }),
    /duplicate invariant/,
  )
})
