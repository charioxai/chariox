import assert from "node:assert/strict"
import test from "node:test"

import {
  DrillChaosClock,
  DrillChaosProcess,
  DrillChaosRandom,
  DrillChaosTrace,
  DrillChaosTransport,
} from "./drill-deterministic-chaos.mjs"

test("seeded chaos random is stable and seed-sensitive", () => {
  const first = new DrillChaosRandom("seed-a")
  const second = new DrillChaosRandom("seed-a")
  const different = new DrillChaosRandom("seed-b")
  const firstValues = Array.from({ length: 8 }, () => first.nextUint32())

  assert.deepEqual(firstValues, Array.from({ length: 8 }, () => second.nextUint32()))
  assert.notDeepEqual(firstValues, Array.from({ length: 8 }, () => different.nextUint32()))
  assert.deepEqual(new DrillChaosRandom("shuffle").shuffle([1, 2, 3, 4]), [1, 3, 4, 2])
})

test("virtual clock suppresses callbacks from dead process generations", async () => {
  const clock = new DrillChaosClock()
  const trace = new DrillChaosTrace(() => clock.nowMs())
  clock.trace = trace
  const process = new DrillChaosProcess({ name: "worker", clock, trace })
  const callbacks = []

  process.schedule("stale", 5, () => callbacks.push("stale"))
  clock.setTimeout(() => process.kill("test"), 1)
  clock.setTimeout(() => {
    process.restart()
    process.schedule("fresh", 1, () => callbacks.push("fresh"))
  }, 2)
  await clock.runUntilIdle()

  assert.deepEqual(callbacks, ["fresh"])
  assert.equal(process.staleCallbacksSuppressed, 1)
  assert.equal(process.pendingCount(), 0)
  assert.equal(clock.pendingCount(), 0)
})

test("programmable transport applies repeated drops, delay, duplication, reorder, and partition", async () => {
  const clock = new DrillChaosClock()
  const trace = new DrillChaosTrace(() => clock.nowMs())
  clock.trace = trace
  const deliveries = []
  const transport = new DrillChaosTransport({
    name: "test-transport",
    clock,
    trace,
    maxQueueDepth: 20,
    deliver: (packet, metadata) => deliveries.push(`${packet.operationId}:${metadata.copyIndex}`),
    faultPlan: [
      { id: "drop", kind: "drop", match: { operationId: "drop" }, times: 2 },
      { id: "delay", kind: "delay", match: { operationId: "delay" }, delayMs: 4 },
      { id: "duplicate", kind: "duplicate", match: { operationId: "duplicate" }, copies: 3 },
      { id: "reorder", kind: "reorder", match: { channel: "event", to: "tui" }, times: 2, window: 2 },
    ],
  })

  transport.partition({ from: "kernel", to: "web" })
  transport.send(packet("partition", "kernel", "web"))
  transport.send(packet("drop"))
  transport.send(packet("drop"))
  transport.send(packet("drop"))
  transport.send(packet("delay"))
  transport.send(packet("duplicate"))
  transport.send(packet("reorder-a", "kernel", "tui", "event"))
  transport.send(packet("reorder-b", "kernel", "tui", "event"))
  transport.reconnect({ from: "kernel", to: "web", release: "deliver" })
  await clock.runUntilIdle()

  assert.deepEqual(deliveries, [
    "drop:0",
    "duplicate:0",
    "duplicate:1",
    "duplicate:2",
    "reorder-b:0",
    "reorder-a:0",
    "partition:0",
    "delay:0",
  ])
  assert.equal(transport.pendingCount(), 0)
  assert(transport.maxObservedQueueDepth > 0)
})

test("drop partitions consume no queue capacity and duplicate enqueue is atomic", async () => {
  const clock = new DrillChaosClock()
  const trace = new DrillChaosTrace(() => clock.nowMs())
  clock.trace = trace
  const deliveries = []
  const transport = new DrillChaosTransport({
    name: "bounded-transport",
    clock,
    trace,
    maxQueueDepth: 1,
    deliver: (value) => deliveries.push(value.operationId),
  })

  transport.send(packet("scheduled"))
  transport.partition({ from: "kernel", to: "web", mode: "drop" })
  assert.doesNotThrow(() => transport.send(packet("partition-drop", "kernel", "web")))
  assert.equal(transport.pendingCount(), 1)
  await clock.runUntilIdle()
  assert.deepEqual(deliveries, ["scheduled"])

  const duplicateTransport = new DrillChaosTransport({
    name: "atomic-duplicate-transport",
    clock,
    trace,
    maxQueueDepth: 2,
    deliver: (value) => deliveries.push(value.operationId),
    faultPlan: [{
      id: "too-many-copies",
      kind: "duplicate",
      match: { operationId: "duplicate" },
      copies: 3,
    }],
  })
  assert.throws(() => duplicateTransport.send(packet("duplicate")), /queue exceeded 2/)
  assert.throws(() => duplicateTransport.send(packet("duplicate")), /queue exceeded 2/)
  assert.equal(duplicateTransport.pendingCount(), 0)
})

function packet(operationId, from = "client", to = "kernel", channel = "request") {
  return {
    messageId: `${operationId}-message`,
    operationId,
    from,
    to,
    channel,
    type: "test",
    payload: {},
  }
}
