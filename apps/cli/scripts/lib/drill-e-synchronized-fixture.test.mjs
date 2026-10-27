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
  try {
    fixture.beforePhase("mutations")
    const pending = fetch(`http://127.0.0.1:${fixture.port}/ready/mutations/a`)
    while (fixture.evidence().mutations.ready.length === 0) await new Promise(resolve => setTimeout(resolve, 5))
    fixture.close()
    assert.equal((await pending).status, 409)
    assert.equal(fixture.evidence().mutations.releaseReason, "cleanup")
  } finally { await fixture.stop() }
})

test("MP-08/MP-10: click hold safety starts at observed admission and releases after takeover", async () => {
  const fixture = await startDrillESynchronizedFixture({ actors: ["a", "b", "c"] })
  try {
    fixture.beforePhase("mutations")
    fixture.tick({ actions: [{ kind: "browser_status", state: "running" }] })
    assert.equal(fixture.evidence().mutations.pageHolds.length, 0)
    assert.equal((await (await fetch(`http://127.0.0.1:${fixture.port}/state`)).json()).held, true)
    fixture.tick({ actions: [{ kind: "click", state: "running" }] })
    fixture.tick({ actions: [{ kind: "click", state: "running" }] })
    assert.equal(fixture.evidence().mutations.pageHolds.length, 1)
    fixture.observed("mutations", ["a", "b", "c"].map(action_id => ({ action_id })))
    assert.equal((await (await fetch(`http://127.0.0.1:${fixture.port}/state`)).json()).held, false)
  } finally { await fixture.stop() }
})

test("MP-08/MP-10: page holds release through pushed events without background timers", async () => {
  const fixture = await startDrillESynchronizedFixture({ actors: ["a", "b", "c"] })
  let reader
  try {
    const response = await fetch(`http://127.0.0.1:${fixture.port}/events`)
    assert.equal(response.headers.get("content-type"), "text/event-stream")
    reader = response.body.getReader()
    const next = async () => new TextDecoder().decode((await reader.read()).value)
    assert.match(await next(), /"held":false/)
    fixture.beforePhase("mutations")
    assert.match(await next(), /"held":true/)
    fixture.observed("mutations", ["a", "b", "c"].map(action_id => ({ action_id })))
    assert.match(await next(), /"held":false/)
  } finally { await reader?.cancel(); await fixture.stop() }
})
