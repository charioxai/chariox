import assert from "node:assert/strict"
import test from "node:test"
import { createWaitingRoomRetainedRows } from "./waiting-room-retained-rows.js"

test("stable IDs survive short reconnects, expire after grace, and live updates replace by ID", () => {
  let now = 0
  const rows = createWaitingRoomRetainedRows<{ id: string; name: string }>(row => row.id, () => now)
  rows.reconcile([{ id: "one", name: "old" }])
  assert.equal(rows.reconcile([])[0]?.displayFreshness, "reconnecting")
  now = 29_999
  assert.equal(rows.reconcile([]).length, 1)
  assert.deepEqual(rows.reconcile([{ id: "one", name: "new" }]), [{ id: "one", name: "new" }])
  rows.reconcile([])
  now += 30_000
  assert.deepEqual(rows.reconcile([]), [])
})

test("cache is visibly stale until an authoritative list replaces it", () => {
  const rows = createWaitingRoomRetainedRows<{ id: string }>(row => row.id, () => 0)
  assert.equal(rows.restore([{ id: "cached" }])[0]?.displayFreshness, "cached/refreshing")
  assert.deepEqual(rows.reconcile([{ id: "live" }]), [{ id: "live" }])
  rows.clear()
  assert.deepEqual(rows.reconcile([]), [])
})
