import assert from "node:assert/strict"
import test from "node:test"

import {
  countDrillAggregateEntriesBy,
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  formatDrillAggregateNextActionSourceDetails,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"

test("counts and orders aggregate next actions", () => {
  const counts = new Map()
  countDrillAggregateNextAction(counts, {
    owner: "runtime-network",
    classification: "relay-runtime",
    nextAction: "inspect relay",
    sourceDetails: [{ source: "remote-matrix", matrix: "remote", scenarioId: "remote", reportPath: "/tmp/remote.json" }],
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
  countDrillAggregateNextAction(counts, {
    owner: "runtime-network",
    classification: "relay-runtime",
    nextAction: "inspect relay",
    count: 3,
    sourceDetails: [{ source: "hetzner-matrix", matrix: "remote", scenarioId: "hetzner", reportPath: "/tmp/remote.json" }],
  })

  assert.deepEqual(formatDrillAggregateNextActionCounts(counts), [
    {
      owner: "runtime-network",
      classification: "relay-runtime",
      nextAction: "inspect relay",
      count: 4,
      sourceDetails: [
        { source: "hetzner-matrix", matrix: "remote", scenarioId: "hetzner", reportPath: "/tmp/remote.json" },
        { source: "remote-matrix", matrix: "remote", scenarioId: "remote", reportPath: "/tmp/remote.json" },
      ],
    },
    {
      owner: "provider-account",
      classification: "provider-auth",
      nextAction: "refresh provider login",
      count: 2,
    },
  ])
  assert.equal(
    formatDrillAggregateNextActionSourceDetails(formatDrillAggregateNextActionCounts(counts)[0].sourceDetails),
    "hetzner-matrix report=/tmp/remote.json, remote-matrix report=/tmp/remote.json",
  )
  assert.equal(
    formatDrillAggregateNextActionSourceDetails([{ reportPath: "/tmp/report-only.json" }]),
    "/tmp/report-only.json",
  )
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
  assert.throws(() => validateDrillAggregateNextAction({
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "inspect state",
    count: 1.5,
  }, "action"), /invalid count/)
  assert.throws(() => validateDrillAggregateNextAction({
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "use Bearer abcdefghijklmnopqrstuvwxyz",
    count: 1,
  }, "action"), /secret-looking nextAction/)
  assert.throws(() => validateDrillAggregateNextAction({
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "inspect state",
    count: 1,
    sourceDetails: [{ source: "sk-secretsecretsecretsecret" }],
  }, "action"), /sourceDetails\[0\]\.source includes secret-looking diagnostic text/)
})

test("aggregate next action counting rejects incomplete actions", () => {
  const counts = new Map()

  assert.throws(() => countDrillAggregateNextAction(counts, {
    owner: "runtime-state",
    classification: "",
    nextAction: "inspect state",
  }), /missing classification/)
  assert.throws(() => countDrillAggregateNextAction(counts, {
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "inspect state",
    count: 1.5,
  }), /invalid count/)
  assert.throws(() => countDrillAggregateNextAction(counts, {
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "use sk-secretsecretsecretsecret",
  }), /secret-looking nextAction/)
  assert.throws(() => countDrillAggregateNextAction(counts, {
    owner: "runtime-state",
    classification: "runtime-timeout",
    nextAction: "inspect state",
    sourceDetails: "bad",
  }), /invalid sourceDetails/)
  assert.equal(counts.size, 0)
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
