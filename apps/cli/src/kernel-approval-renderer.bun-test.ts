import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable, TextRenderable, TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createKernelApprovalRenderer } from "./kernel-approval-renderer.js"
import { createKernelApprovalController, type KernelApprovalView } from "./kernel-approval-controller.js"
import type { RuntimeSession } from "./cli-types.js"

const view: KernelApprovalView = {
  open: false, count: 1, index: 0, selected: null, pending: false, connected: true, error: null,
  interaction: {
    id: "approval-1", kernel_operation_id: "install-1", kind: "permission", level: "warning",
    title: "Install Linear", message: "Allow this App to use the capabilities listed in the installation?",
    choices: [{ id: "deny", label: "Deny", reply: "deny" }, { id: "allow", label: "Allow", reply: "allow" }],
    requested_at_ms: 1,
  },
}

test("global approvals render over a zero-agent workspace and preserve the sole prompt", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(new TextRenderable(harness.renderer, { content: "App workspace · no focus agent\nPrompt > draft kept", top: 22 }))
  harness.renderer.root.add(box)
  const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose() {} })
  surface.assign(box)
  try {
    surface.render(view, { width: 80, height: 24 })
    await harness.renderOnce()
    let frame = harness.captureCharFrame()
    assert.match(frame, /Chariox · 1 approval · F8/)
    assert.doesNotMatch(frame, /Install Linear/)
    surface.render({ ...view, open: true }, { width: 80, height: 24 })
    await harness.renderOnce()
    frame = harness.captureCharFrame()
    assert.match(frame, /Install Linear/)
    assert.match(frame, /Enter confirm/)
    assert.match(frame, /Prompt > draft kept/)
    assert.doesNotMatch(frame, /› Allow|› Deny/)
    surface.render({ ...view, open: true, selected: 1, pending: true }, { width: 80, height: 24 })
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /Waiting for kernel confirmation/)
    surface.render({ ...view, open: true, connected: false }, { width: 80, height: 24 })
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /Disconnected · reconnect to respond/)
  } finally { harness.renderer.destroy() }
})

test("approval title, text and choice controls fit narrow terminals without horizontal clipping", async () => {
  const harness = await createTestRenderer({ width: 48, height: 18, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose() {} })
  surface.assign(box)
  try {
    surface.render({ ...view, open: true, selected: 0 }, { width: 48, height: 18 })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Install Linear/)
    assert.match(frame, /› Deny/)
    assert.match(frame, /Enter confirm/)
    assert.match(frame, /scroll/)
  } finally { harness.renderer.destroy() }
})

test("actual OpenTUI keyboard delivery isolates approval choices from the focused textarea", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  let submitted = 0
  const requests: string[] = []
  const prompt = new TextareaRenderable(harness.renderer, {
    initialValue: "draft kept", keyBindings: [{ name: "return", action: "submit" }],
    onSubmit: () => { if (!controller.ownsInput()) submitted += 1 },
  })
  harness.renderer.root.add(prompt)
  const controller = createKernelApprovalController({
    getSession: () => ({ id: "session-1", agents: [], active_interactions: [view.interaction!] }) as unknown as RuntimeSession,
    connected: () => true, onView() {}, scroll() {},
    onOpen: () => prompt.blur(), onClose: () => prompt.focus(),
    respond: async (_session, _interaction, choice) => { requests.push(choice); return new Promise(() => {}) },
    applySession() {},
  })
  harness.renderer.keyInput.on("keypress", controller.handleKey)
  try {
    controller.sync()
    prompt.focus()
    await harness.mockInput.pressKeys(["RETURN"])
    assert.equal(submitted, 1)
    assert.equal(requests.length, 0)
    await harness.mockInput.pressKeys(["F8", "RETURN", "1"])
    assert.equal(prompt.focused, false)
    assert.equal(requests.length, 0)
    assert.equal(submitted, 1)
    assert.equal(prompt.plainText, "draft kept")
    await harness.mockInput.pressKeys(["ARROW_DOWN", "RETURN", "RETURN"])
    assert.deepEqual(requests, ["deny"])
    await harness.mockInput.pressKeys(["ESCAPE"])
    // A bare ESC is held briefly to distinguish it from a terminal sequence.
    await new Promise((resolve) => setTimeout(resolve, 50))
    assert.equal(prompt.focused, true)
    assert.equal(prompt.plainText, "draft kept")
  } finally {
    harness.renderer.keyInput.off("keypress", controller.handleKey)
    controller.dispose()
    harness.renderer.destroy()
  }
})

test("actual mouse clicks require the primary button and a connected, nonpending approval", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const box = new BoxRenderable(harness.renderer, { position: "absolute", left: 0, top: 0 })
  harness.renderer.root.add(box)
  const choices: string[] = []
  const surface = createKernelApprovalRenderer(harness.renderer, { show() {}, choose: (interactionId, id) => { assert.equal(interactionId, "approval-1"); choices.push(id) } })
  surface.assign(box)
  try {
    surface.render({ ...view, open: true }, { width: 80, height: 24 })
    await harness.renderOnce()
    const lines = harness.captureCharFrame().split("\n")
    const y = lines.findIndex((line) => line.includes("Deny"))
    const x = lines[y]!.indexOf("Deny")
    assert.ok(x >= 0 && y >= 0)
    await harness.mockMouse.click(x, y, 2)
    assert.deepEqual(choices, [])
    await harness.mockMouse.click(x, y, 0)
    assert.deepEqual(choices, ["deny"])
    for (const state of [{ pending: true }, { connected: false }]) {
      surface.render({ ...view, open: true, ...state }, { width: 80, height: 24 })
      await harness.renderOnce()
      await harness.mockMouse.click(x, y, 0)
    }
    assert.deepEqual(choices, ["deny"])
    surface.render({ ...view, open: true, selected: 1,
      interaction: { ...view.interaction!, message: "Long permission details\n".repeat(100) },
    }, { width: 80, height: 24 })
    await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /Selected: Allow/)
  } finally { harness.renderer.destroy() }
})
