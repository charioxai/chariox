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
  const controller = createProviderAccountRecoveryController({ renderer: h.renderer, dimensions: () => ({ width: 100, height: 40 }), actions: () => [
    { label: "Open Provider Accounts", run: () => invoked.push("accounts") },
    { label: "Switch account", run: () => invoked.push("switch") },
    { label: "Retry", run: () => invoked.push("retry") },
  ] })
  try {
    controller.assignBox(box)
    controller.show("codex-1 needs sign-in. Sign in through Provider Accounts, or select signed-in account chariox.")
    await h.renderOnce()
    const frame = h.captureCharFrame()
    assert.match(frame, /Provider account needs sign-in/)
    assert.match(frame, /chariox/)
    assert.match(frame, /Open Provider Accounts/)
    controller.handleKey({ name: "down" })
    controller.handleKey({ name: "return" })
    assert.deepEqual(invoked, ["switch"])
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
