import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"
import { createRoot } from "solid-js"
import { createTestRenderer } from "@opentui/core/testing"
import { TextareaRenderable } from "@opentui/core"
import { createCliAppActionsComposition } from "./cli-app-actions-composition.js"

test("DEV refresh failure closes the panel, restores the draft and retains recovery advice in the transcript", async () => {
  const h = await createTestRenderer({width: 100, height: 24, useThread: false})
  const prompt = new TextareaRenderable(h.renderer, {initialValue: "draft kept"})
  h.renderer.root.add(prompt)
  prompt.focus()
  const notices: string[] = [], flashes: unknown[] = [], requests: unknown[] = []
  const message = "The App release is unavailable or no longer trusted; check /app status"
  let fail = false, dispose!: () => void
  const client = {send: async (request: unknown) => {
    requests.push(request)
    if (fail) throw new Error(message)
    return {AppViewInspected: {installation_id: "dev", target_id: "tab-2", generation: "1", development: true,
      catalog_json: '{"actions":[],"registrations":[],"state":{"ok":true},"warnings":[]}'}}
  }}
  const deps = {
    client: () => client, session: () => "room", renderer: h.renderer,
    dimensions: () => ({width: 100, height: 24}), themeRevision: () => 0,
    currentFocus: () => prompt, promptFocus: () => prompt, closeOtherDialog() {},
    appendNotice: (text: string) => {notices.push(text)},
    flashFooter: (text: string, tone: "info" | "error") => {flashes.push({text, tone})},
  }
  const controller = createRoot(cleanup => {dispose = cleanup; return createCliAppActionsComposition(deps)})
  h.renderer.keyInput.on("keypress", controller.handleKey)
  try {
    await controller.handle(["actions", "toggle"])
    await h.renderOnce()
    assert.match(h.captureCharFrame(), /As the agent sees it/)
    fail = true
    h.mockInput.pressKey("r")
    await sleep(20)
    await h.renderOnce()
    assert.equal(controller.ownsInput(), false)
    assert.equal(prompt.focused, true)
    assert.equal(prompt.plainText, "draft kept")
    assert.deepEqual(requests, [{InspectAppView: {session_id: "room"}}, {InspectAppView: {session_id: "room", target_id: "tab-2"}}])
    assert.deepEqual(notices, [message])
    assert.deepEqual(flashes, [{text: "App view inspection failed; see transcript", tone: "error"}])
  } finally {
    h.renderer.keyInput.off("keypress", controller.handleKey)
    dispose()
    h.renderer.destroy()
  }
})
