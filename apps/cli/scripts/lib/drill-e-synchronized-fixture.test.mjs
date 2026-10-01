import assert from "node:assert/strict"
import test from "node:test"
import { startDrillESynchronizedFixture } from "./drill-e-synchronized-fixture.mjs"

test("MP-08/MP-10: HTTP fixture primes pages, gates three providers, and releases on measured actions", async () => {
  const fixture = await startDrillESynchronizedFixture({ actors: ["a", "b", "c"] })
  const base = `http://127.0.0.1:${fixture.port}`
  try {
    assert.equal((await (await fetch(`${base}/page/same`)).text()).match(/<button>/g).length, 30_000)
    fixture.beforePhase("reads")
    const arrivals = ["a", "b", "c"].map(actor => fetch(`${base}/ready/reads/${actor}`))
    assert.ok((await Promise.all(arrivals)).every(response => response.ok))
    assert.equal((await fetch(`${base}/ready/reads/a`)).status, 409)
    const held = fetch(`${base}/page/other`)
    while (fixture.evidence().reads.pageHolds.length === 0) await new Promise(resolve => setTimeout(resolve, 5))
    fixture.observed("reads", ["read-a", "read-b", "work-c"].map(action_id => ({ action_id })))
    assert.equal((await held).status, 200)
    assert.equal(fixture.evidence().reads.pageReleaseReason, "observed_overlap")
    assert.ok(fixture.promptPrefix("reads", "a").includes("python3 -c"))
  } finally { await fixture.stop() }
})

test("MP-08/MP-10: closing fixture unblocks incomplete provider preparation", async () => {
  const fixture = await startDrillESynchronizedFixture({ actors: ["a", "b", "c"] })
  fixture.beforePhase("mutations")
  const pending = fetch(`http://127.0.0.1:${fixture.port}/ready/mutations/a`)
  while (fixture.evidence().mutations.ready.length === 0) await new Promise(resolve => setTimeout(resolve, 5))
  fixture.close()
  assert.equal((await pending).status, 200)
  assert.equal(fixture.evidence().mutations.releaseReason, "cleanup")
  await fixture.stop()
})
