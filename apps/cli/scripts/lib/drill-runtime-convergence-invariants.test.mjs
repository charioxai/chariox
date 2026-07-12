import assert from "node:assert/strict"
import test from "node:test"

import {
  assertDrillRuntimeConvergenceInvariants,
  evaluateDrillRuntimeConvergenceInvariants,
} from "./drill-runtime-convergence-invariants.mjs"

test("runtime convergence invariants accept a fully drained converged model", () => {
  const report = evaluateDrillRuntimeConvergenceInvariants(fixture())

  assert.equal(report.status, "passed")
  assert.equal(report.checks.length, 8)
  assert.doesNotThrow(() => assertDrillRuntimeConvergenceInvariants(report))
})

test("runtime convergence invariants expose every failed property", () => {
  const value = fixture()
  value.executionCounts = { "op-1": 2, "op-2": 0 }
  value.clientStates.web = { cursor: 1, operationIds: ["op-1"], values: { prompt: "one" } }
  value.cursorHistory.web = [0, 2, 1]
  value.authorityMutations.push({ owner: "ui-client", operationId: "op-2" })
  value.queue = { limit: 1, maxObserved: 2, pending: 1 }
  value.resources = { clockTasks: 1 }
  value.staleCallbacks = { scheduled: 1, suppressed: 0, executed: 1 }

  const report = evaluateDrillRuntimeConvergenceInvariants(value)
  assert.equal(report.status, "failed")
  assert.deepEqual(report.checks.filter((item) => !item.ok).map((item) => item.id), [
    "bounded-queues",
    "eventual-client-convergence",
    "monotonic-cursors",
    "no-action-loss",
    "no-duplicate-execution",
    "resource-cleanup",
    "stale-callback-suppression",
    "valid-authority",
  ])
  assert.throws(() => assertDrillRuntimeConvergenceInvariants(report), /runtime chaos invariants failed/)
})

test("runtime convergence preserves canonical operation ordering", () => {
  const value = fixture()
  value.clientStates.web.operationIds.reverse()

  const report = evaluateDrillRuntimeConvergenceInvariants(value)
  assert.equal(report.checks.find((item) => item.id === "eventual-client-convergence").ok, false)
})

test("runtime convergence rejects execution that was never accepted", () => {
  const value = fixture()
  value.executionCounts.set("op-unaccepted", 1)
  value.authorityMutations.push({ owner: "kernel-authority", operationId: "op-unaccepted" })

  const report = evaluateDrillRuntimeConvergenceInvariants(value)
  assert.equal(report.checks.find((item) => item.id === "no-action-loss").ok, false)
  assert.equal(report.checks.find((item) => item.id === "valid-authority").ok, false)
})

function fixture() {
  const canonical = {
    cursor: 2,
    operationIds: ["op-1", "op-2"],
    values: { prompt: "one", workflow: "two" },
  }
  return {
    acceptedOperations: new Set(["op-1", "op-2"]),
    executionCounts: new Map([["op-1", 1], ["op-2", 1]]),
    kernelState: canonical,
    clientStates: {
      tui: structuredClone(canonical),
      web: structuredClone(canonical),
    },
    cursorHistory: {
      tui: [0, 1, 2],
      web: [0, 2],
    },
    authorityMutations: [
      { owner: "kernel-authority", operationId: "op-1" },
      { owner: "kernel-authority", operationId: "op-2" },
    ],
    queue: { limit: 8, maxObserved: 4, pending: 0 },
    resources: { clockTasks: 0 },
    staleCallbacks: { scheduled: 1, suppressed: 1, executed: 0 },
  }
}
