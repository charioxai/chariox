import assert from "node:assert/strict"
import test from "node:test"
import { createDrillEBarrier } from "./drill-e-barrier.mjs"

test("MP-08/MP-10: readiness requires each exact actor and cannot release on duplicate arrivals", async () => {
  const gate = createDrillEBarrier({ actors: ["a", "b", "c"] })
  let released = false
  const a = gate.arrive("a").then(() => { released = true })
  assert.throws(() => gate.arrive("a"), /duplicate/)
  assert.throws(() => gate.arrive("foreign"), /actor/)
  const b = gate.arrive("b")
  await Promise.resolve()
  assert.equal(released, false)
  const c = gate.arrive("c")
  await Promise.all([a, b, c])
  assert.equal(gate.evidence().ready.length, 3)
  assert.equal(gate.evidence().releaseReason, "all_actors_ready")
})

test("MP-08/MP-10: page holds release only on measured overlap or bounded safety failure", async () => {
  let safety
  let cleared = false
  const gate = createDrillEBarrier({ actors: ["a", "b", "c"],
    schedule: (fn, ms) => { assert.equal(ms, 4000); safety = fn; return 1 },
    unschedule: () => { cleared = true },
  })
  let released = false
  const held = gate.hold("same").then(() => { released = true })
  await Promise.resolve()
  assert.equal(released, false)
  assert.throws(() => gate.releasePages("observed_overlap", []), /action/)
  gate.releasePages("observed_overlap", ["read-a", "read-b", "work-c"])
  await held
  assert.equal(cleared, true)
  assert.deepEqual(gate.evidence().overlapActionIds, ["read-a", "read-b", "work-c"])
  assert.equal(gate.evidence().pageReleaseReason, "observed_overlap")

  const failed = createDrillEBarrier({ actors: ["a", "b", "c"],
    schedule: (fn) => { safety = fn; return 1 }, unschedule: () => {},
  })
  const pending = failed.hold("other")
  safety()
  await pending
  assert.equal(failed.evidence().pageReleaseReason, "safety_deadline")
  assert.deepEqual(failed.evidence().overlapActionIds, [])
})

test("MP-08/MP-10: cleanup releases every readiness and page waiter", async () => {
  const gate = createDrillEBarrier({ actors: ["a", "b", "c"] })
  const pending = [gate.arrive("a"), gate.hold("same"), gate.hold("other")]
  gate.close()
  await Promise.all(pending)
  assert.equal(gate.evidence().releaseReason, "cleanup")
  assert.equal(gate.evidence().pageReleaseReason, "cleanup")
})
