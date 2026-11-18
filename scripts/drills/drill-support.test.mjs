// Run: node --test scripts/drills/drill-support.test.mjs (needs `pnpm build:kernel-client`; the stand-in kernel lives under /root/.chariox/dev/, the drills' state-root rule).
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmodSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { test } from "node:test"
import { DEFAULT_KINDS, wakeTurns, wokeReply } from "./ack-drop-support.mjs"
import { answerWhilePending, startDrillKernel } from "./own-kernel.mjs"

const turn = (text, extra = {}) => ({ user_prompt: { entry: { text } }, entries: [], ...extra })
const ask = turn('Call the chariox events_timer tool with delay_ms 60000 and label "ACKDROP-X". Then reply ACKDROP-X_WOKE.')
const wake = reply => turn("Inbox: at least 1 new event (1 timer). Read it with chariox.events.inbox after 7, then chariox.events.ack each one you handled.", { system_prompt: true, prompt_origin: "chariox", entries: [{ entry: { kind: "provider_output", text: reply } }] })

test("the default drop kinds arm what an events_timer fires (FireWakes -> wake_intent)", () => {
  assert.equal(DEFAULT_KINDS[0], "wake_intent")
  const source = readFileSync(new URL("../../apps/kernel/src/durable_state/agent_lifecycle/faults.rs", import.meta.url), "utf8")
  for (const kind of DEFAULT_KINDS) assert.ok(source.includes(`Some("${kind}")`), `${kind} is a delivery kind`)
  assert.match(source, /Operation::FireWakes \{ \.\. \} => Some\("wake_intent"\)/)
})

test("a kernel wake prompt, which carries counts and no timer label, is found and its reply read", () => {
  const turns = [ask, wake("ACKDROP-X_WOKE")]
  assert.equal(wakeTurns(turns).length, 1)
  assert.equal(wokeReply(wakeTurns(turns), "ACKDROP-X_WOKE"), true)
  assert.equal(wokeReply([wake("something else")], "ACKDROP-X_WOKE"), false)
})

test("the drill's own request is not a wake", () => {
  assert.equal(wakeTurns([ask]).length, 0)
})

test("a card is answered while its authorizing request is still pending", async () => {
  let answered = false
  const request = new Promise(resolve => { const poll = setInterval(() => { if (answered) { clearInterval(poll); resolve("authorized") } }, 5) })
  const result = await Promise.race([
    answerWhilePending({ request, findCard: async () => ({ id: "card" }), answer: async card => { assert.equal(card.id, "card"); answered = true } }),
    new Promise(resolve => setTimeout(() => resolve("deadlocked"), 2000)),
  ])
  assert.equal(result, "authorized")
})

test("a kernel that never becomes ready is stopped by its exact pid", { timeout: 90000 }, async t => {
  const root = join("/root/.chariox/dev", `drill-support-test-${process.pid}`)
  mkdirSync(root, { recursive: true })
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const pidFile = join(root, "kernel.pid")
  const binary = join(root, "stand-in-kernel")
  writeFileSync(binary, `#!/bin/sh\necho $$ > ${pidFile}\nexec sleep 300\n`)
  chmodSync(binary, 0o755)
  await assert.rejects(startDrillKernel({ label: "standin", binary, stateRoot: root, evidence: join(root, "evidence"), profilePath: root, readyTimeoutMs: 1500 }), /timeout: kernel ready/)
  const pid = Number(readFileSync(pidFile, "utf8"))
  assert.ok(pid > 1)
  const alive = spawnSync("kill", ["-0", String(pid)]).status === 0
  if (alive) process.kill(pid, "SIGKILL")
  assert.equal(alive, false, "the spawned kernel is gone after the failed start")
})
