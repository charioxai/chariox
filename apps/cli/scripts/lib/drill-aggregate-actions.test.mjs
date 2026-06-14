import assert from "node:assert/strict"
import test from "node:test"

import {
  countDrillAggregateEntriesBy,
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"

test("counts and orders aggregate next actions", () => {
  const counts = new Map()
  countDrillAggregateNextAction(counts, {
    owner: "runtime-network",
    classification: "relay-runtime",
    nextAction: "inspect relay",
  })
  countDrillAggregateNextAction(counts, {
    owner: "provider-account",
    classification: "provider-auth",
    nextAction: "refresh provider login",
  })
  countDrillAggregateNextAction(counts, {
    owner: "provider-account",
    classification: "provider-auth",
    nextAction: "refresh provider login",
  })

  assert.deepEqual(formatDrillAggregateNextActionCounts(counts), [
    {
      owner: "provider-account",
      classification: "provider-auth",
      nextAction: "refresh provider login",
      count: 2,
    },
    {
      owner: "runtime-network",
      classification: "relay-runtime",
      nextAction: "inspect relay",
      count: 1,
    },
  ])
})

test("validates aggregate next action entries", () => {
  validateDrillAggregateNextAction({
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "inspect state",
    count: 1,
  }, "action")

  assert.throws(() => validateDrillAggregateNextAction({
    owner: "runtime-state",
    classification: "",
    nextAction: "inspect state",
    count: 1,
  }, "action"), /missing classification/)
  assert.throws(() => validateDrillAggregateNextAction({
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "inspect state",
    count: 0,
  }, "action"), /invalid count/)
})

test("counts aggregate entries by stable sorted key", () => {
  assert.deepEqual(countDrillAggregateEntriesBy([
    { owner: "runtime-network" },
    { owner: "provider-account" },
    { owner: "provider-account" },
  ], (entry) => entry.owner), {
    "provider-account": 2,
    "runtime-network": 1,
  })
})
