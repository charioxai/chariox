import assert from "node:assert/strict"
import test from "node:test"
import { createManagedParityObserverHooks } from "./managed-browser-computer-parity-observer-hooks.mjs"

test("observation failure stops admission but preserves bounded retirement observation and cleanup", async () => {
  const events = []
  const hooks = createManagedParityObserverHooks({ timeoutMs: 10 })
  hooks.bind({
    async observeCreated(receipt) { events.push(receipt); throw new Error("census unavailable") },
    async beforeRetire(receipt) { events.push(receipt); throw new Error("still unavailable") },
  })
  hooks.admit()
  await assert.rejects(hooks.observe("observeCreated", { sliceId: "slice-1" }), /census unavailable/)
  assert.throws(() => hooks.admit(), /census unavailable/)
  assert.doesNotThrow(() => hooks.admit({ cleanup: true }))
  await hooks.observe("beforeRetire", { sliceId: "slice-1" }, { cleanup: true })
  assert.equal(events.length, 2)
  assert.throws(() => hooks.assertHealthy(), /census unavailable/)
})

test("observation is bounded and cannot rewrite the product ownership receipt", async () => {
  const hooks = createManagedParityObserverHooks({ timeoutMs: 5 })
  const receipt = { sliceId: "slice-1" }
  let observedSignal
  hooks.bind({
    observeCreated(value, { signal }) { value.sliceId = "foreign"; observedSignal = signal; return new Promise(() => {}) },
    async beforeRetire() {},
  })
  await assert.rejects(hooks.observe("observeCreated", receipt), /deadline/)
  assert.equal(receipt.sliceId, "slice-1")
  assert.equal(observedSignal.aborted, true)
  assert.throws(() => hooks.bind({}), /once before product admission/)
})
