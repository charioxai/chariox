import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createProviderAccountRecoveryController } from "./provider-account-recovery-controller.js"

test("MP-08 / MP-11 account recovery shows the full reason and consumes duplicate Enter", async () => {
  const h = await createTestRenderer({ width: 100, height: 40, useThread: false })
  const box = new BoxRenderable(h.renderer, { position: "absolute", left: 0, top: 0 })
  h.renderer.root.add(box)
  const invoked: string[] = []
  const targets: unknown[] = []
  const controller = createProviderAccountRecoveryController({ renderer: h.renderer, dimensions: () => ({ width: 100, height: 40 }), actions: target => { targets.push(target); return [
    { label: "Open Provider Accounts", run: () => invoked.push("accounts") },
    { label: "Switch account", run: () => invoked.push("switch") },
    { label: "Retry", run: () => invoked.push("retry") },
  ] } })
  try {
    controller.assignBox(box)
    controller.show("codex-1 needs sign-in. Sign in through Provider Accounts, or select signed-in account chariox.", {sessionId: "session-1", agentId: "agent-b"})
    await h.renderOnce()
    const frame = h.captureCharFrame()
    assert.match(frame, /Provider account needs sign-in/)
    assert.match(frame, /chariox/)
    assert.match(frame, /Open Provider Accounts/)
    controller.handleKey({ name: "down" })
    controller.handleKey({ name: "return" })
    assert.deepEqual(invoked, ["switch"])
    assert.deepEqual(targets.at(-1), {sessionId: "session-1", agentId: "agent-b"})
    assert.equal(controller.ownsInput(), true)
    assert.equal(controller.handleKey({ name: "return" }), true)
    assert.deepEqual(invoked, ["switch"])
    await new Promise(resolve => setTimeout(resolve, 10))
    assert.equal(controller.ownsInput(), false)
    controller.show("chariox has exhausted its allowance; resets 2026-10-10 12:00 UTC. Select another signed-in account with quota.")
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /Choose provider account with quota/)
    assert.match(h.captureCharFrame(), /2026-10-10 12:00 UTC/)
  } finally { controller.close(); h.renderer.destroy() }
})

test("MP-08/MP-10/MP-11 kernel quota refusals expose account recovery and quota heading", async () => {
  const h = await createTestRenderer({width:100,height:40,useThread:false})
  const box = new BoxRenderable(h.renderer, {position:"absolute",left:0,top:0})
  h.renderer.root.add(box)
  const controller = createProviderAccountRecoveryController({renderer:h.renderer,dimensions:()=>({width:100,height:40}),actions:()=>[
    {label:"Open Provider Accounts",run(){}},{label:"Switch account",run(){}},{label:"Retry",run(){}},
  ]})
  try {
    controller.assignBox(box)
    for (const usage of ["usage allowance and credits are exhausted", "usage is exhausted"]) {
      const message = `selected codex account b cannot start new work because its ${usage}`
      controller.show(message)
      await h.renderOnce()
      assert.equal(controller.ownsInput(), true)
      const frame = h.captureCharFrame()
      assert.match(frame, /Choose provider account with quota/)
      assert.match(frame, /Open Provider Accounts/)
      assert.match(frame, /Retry/)
      controller.close()
    }
  } finally {controller.close();h.renderer.destroy()}
})

test("MP-08/MP-10/MP-11 no-draft launch refusal exposes and invokes Retry", async () => {
  const h = await createTestRenderer({width:100,height:40,useThread:false})
  const box = new BoxRenderable(h.renderer, {position:"absolute",left:0,top:0})
  h.renderer.root.add(box)
  let retries = 0
  const controller = createProviderAccountRecoveryController({renderer:h.renderer,dimensions:()=>({width:100,height:40}),actions:()=>[]})
  try {
    controller.assignBox(box)
    controller.showLaunchRefusal("working directory does not exist", () => { retries++ })
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /working directory does not exist/)
    assert.match(h.captureCharFrame(), /Retry/)
    assert.doesNotMatch(h.captureCharFrame(), /sign-in/)
    controller.handleKey({name:"return"})
    controller.handleKey({name:"return"})
    assert.equal(retries, 1)
  } finally {controller.close();h.renderer.destroy()}
})

test("MP-08/MP-10/MP-11 attached recovery opens visible choices and retains the rejected target", async () => {
  const h = await createTestRenderer({width:100,height:40,useThread:false})
  const box = new BoxRenderable(h.renderer, {position:"absolute",left:0,top:0})
  h.renderer.root.add(box)
  const invoked: unknown[] = []
  const target = {sessionId: "session-a", agentId: "agent-b"}
  const controller = createProviderAccountRecoveryController({renderer:h.renderer,dimensions:()=>({width:100,height:40}),actions:()=>[]})
  try {
    controller.assignBox(box)
    controller.showAccountChoices("Select an account for the rejected Codex agent.", [
      {label:"Account A · sign-in not checked",run:()=>invoked.push({profile:"a",target})},
      {label:"Account B · needs sign-in",run:()=>invoked.push({profile:"b",target})},
    ])
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /Choose provider account/)
    assert.match(h.captureCharFrame(), /Account A/)
    assert.match(h.captureCharFrame(), /Account B/)
    controller.handleKey({name:"down"})
    controller.handleKey({name:"return"})
    assert.deepEqual(invoked, [{profile:"b",target}])
  } finally {controller.close();h.renderer.destroy()}
})
