import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_CHAOS_CONTRACT_SCHEMA,
  DRILL_CHAOS_FAULT_KINDS,
  DRILL_CHAOS_INVARIANT_IDS,
  drillChaosContractManifest,
  validateDrillChaosFaultPlan,
  validateDrillChaosReplayBundle,
} from "./drill-chaos-contract.mjs"
import { createDeterministicRuntimeChaosReplay } from "./drill-deterministic-runtime-model.mjs"

test("chaos contract exposes every required deterministic fault and invariant", () => {
  assert.deepEqual(drillChaosContractManifest(), {
    schema: DRILL_CHAOS_CONTRACT_SCHEMA,
    replaySchema: "chariox.drill.chaos_replay.v1",
    invariantsSchema: "chariox.drill.chaos_invariants.v1",
    faultKinds: [...DRILL_CHAOS_FAULT_KINDS],
    invariantIds: [...DRILL_CHAOS_INVARIANT_IDS],
  })
  assert.deepEqual(DRILL_CHAOS_FAULT_KINDS, [
    "delay",
    "drop",
    "duplicate",
    "process-death",
    "reorder",
    "route-partition",
    "route-reconnect",
    "stale-callback",
  ])
  assert.deepEqual(DRILL_CHAOS_INVARIANT_IDS, [
    "bounded-queues",
    "eventual-client-convergence",
    "monotonic-cursors",
    "no-action-loss",
    "no-duplicate-execution",
    "resource-cleanup",
    "stale-callback-suppression",
    "valid-authority",
  ])
})

test("chaos contract validates complete replay evidence and rejects drift", async () => {
  const replay = await createDeterministicRuntimeChaosReplay({ seed: "contract-validation" })
  const traceWithoutProcessDeath = replay.trace
    .filter((event) => event.details?.faultId !== "provider-process-death")
    .map((event, index) => ({ ...event, sequence: index + 1 }))
  assert.equal(validateDrillChaosReplayBundle(replay), replay)

  assert.throws(
    () => validateDrillChaosReplayBundle({ ...replay, schema: "chariox.drill.chaos_replay.v2" }),
    /unsupported schema/,
  )
  assert.throws(
    () => validateDrillChaosReplayBundle({
      ...replay,
      trace: traceWithoutProcessDeath,
      summary: {
        ...replay.summary,
        traceEvents: traceWithoutProcessDeath.length,
        faultsApplied: replay.summary.faultsApplied - 1,
      },
    }),
    /faults without trace evidence: provider-process-death/,
  )
  assert.throws(
    () => validateDrillChaosReplayBundle({ ...replay, metadata: { authorization: "do-not-record" } }),
    /sensitive field authorization/,
  )
})

test("chaos fault plan rejects unsupported and duplicate definitions", () => {
  assert.throws(
    () => validateDrillChaosFaultPlan([{ id: "bad", kind: "random-corruption" }]),
    /unsupported kind/,
  )
  assert.throws(
    () => validateDrillChaosFaultPlan([
      { id: "same", kind: "drop" },
      { id: "same", kind: "delay", delayMs: 1 },
    ]),
    /duplicate fault id same/,
  )
})
