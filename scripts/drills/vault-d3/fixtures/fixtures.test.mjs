import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, stat } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { startAll } from "./serve.mjs"

const get = async (url, init) => { const r = await fetch(url, init); return { status: r.status, text: await r.text() } }

test("every fixture serves, the state file is mode 600, and verify/signup/grid checks hold server side", async () => {
  const stateDir = await mkdtemp(join(tmpdir(), "fx-"))
  const all = await startAll({ stateDir, basePort: 18441 + Math.floor(Math.random() * 2000) })
  try {
    const o = all.origins
    for (const [name, origin] of Object.entries(o)) assert.ok(origin.startsWith("http://127.0.0.1:"), name)
    assert.equal(new Set(Object.values(o)).size, 7, "seven distinct origins")
    assert.equal((await stat(join(stateDir, "f2-reveal.needle"))).mode & 0o777, 0o600)
    const revealToken = (await readFile(join(stateDir, "f2-reveal.needle"), "utf8")).trim()
    assert.equal((await get(o.reveal + "/verify?kind=reveal", { method: "POST", headers: { authorization: "Bearer " + revealToken } })).status, 200)
    assert.equal((await get(o.reveal + "/verify?kind=reveal", { method: "POST", headers: { authorization: "Bearer wrong" } })).status, 401)
    const state = await get(o.reveal + "/state")
    assert.ok(!state.text.includes(revealToken), "the verify log holds hashes only")
        const home = (await get(o.reveal + "/")).text
    assert.ok(home.includes('data-hidden="1"') && !home.includes(revealToken), "the token is not in the page before Reveal")
    assert.equal(JSON.parse((await get(o.reveal + "/show?kind=reveal", { method: "POST" })).text).value, revealToken)
    const good = "Aa1!Aa1!Aa1!Aa1!"
    assert.equal((await get(o.signup + "/signup", { method: "POST", body: new URLSearchParams({ email: "a@b.c", password: good, confirm: good }) })).status, 200)
    assert.equal((await get(o.signup + "/signup", { method: "POST", body: new URLSearchParams({ email: "a@b.c", password: good, confirm: "x" }) })).status, 422)
    assert.ok(!JSON.stringify(all.signup.log).includes(good))
    const grid = (await get(o.grid + "/grid.html")).text
    assert.ok(grid.includes("<canvas") && o.grid !== o.human)
    assert.equal((await get(o.grid + "/verify", { method: "POST", body: JSON.stringify({ picked: [] }) })).status, 200)
    assert.equal(all.human.log.at(-1).ok, false)
    assert.equal((await get(o.login + "/small")).status, 200)
  } finally { await all.close(); await rm(stateDir, { recursive: true, force: true }) }
})
