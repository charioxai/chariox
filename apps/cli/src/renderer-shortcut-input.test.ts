import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import test from "node:test"
import type { KeyHandler } from "@opentui/core"
import { bindRendererShortcutInput } from "./renderer-shortcut-input.js"

for (const transition of ["handoff", "dispose"] as const) {
  test(`MP-08 / MP-10 decoded shortcuts pending at ${transition} cannot reach the app`, async () => {
    const input = new EventEmitter()
    let enabled = true, handled = 0, discarded = 0
    const dispose = bindRendererShortcutInput({
      keyInput: input as KeyHandler,
      enabled: () => enabled,
      handleEvent: () => { handled++; return true },
      discardInput: () => { discarded++ },
    })
    input.emit("keypress", { name: "c", meta: true })
    input.emit("paste", {})
    assert.equal(handled, 0, "dispatch must wait for renderer selection/ownership")
    if (transition === "handoff") enabled = false
    else dispose()
    await Promise.resolve()
    assert.equal(handled, 0)
    assert.equal(discarded, transition === "handoff" ? 2 : 0)
    if (transition === "handoff") {
      input.emit("keypress", { name: "c" })
      enabled = true
      await Promise.resolve()
      assert.equal(handled, 0, "input admitted during handoff must stay discarded")
      input.emit("keypress", { name: "c" })
      input.emit("keyrelease", { name: "c", eventType: "release" })
      await Promise.resolve()
      assert.equal(handled, 2)
      dispose()
    }
    assert.deepEqual(input.eventNames(), [])
  })
}
