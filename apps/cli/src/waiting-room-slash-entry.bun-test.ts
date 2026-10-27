import assert from "node:assert/strict"
import test from "node:test"
import { TextareaRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { createWaitingRoomKeyController } from "./waiting-room-key-controller.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

// A fresh waiting room has no focused prompt. The renderer hands the textarea
// every key of a terminal write before our raw stdin listener runs, so the
// first slash has to focus the prompt from the renderer's own key stream:
// the same key, and the rest of the typed or sent command, then reach it.
async function waitingRoom() {
  const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
  const prompt = new TextareaRenderable(harness.renderer, { width: 60, height: 1 })
  harness.renderer.root.add(prompt)
  const activations: string[] = []
  const controller = createWaitingRoomKeyController({
    isAttached: () => false,
    hotkeysOpen: () => false,
    promptFocused: () => prompt.focused,
    commandCenterOpen: () => false,
    commandCenterQuery: () => prompt.plainText,
    getWaitingRoomState: () => ({ focus: "new" }) as WaitingRoomState,
    getSessions: () => [],
    getProviderCatalog: () => fallbackProviderCatalog(),
    getRemoteState: () => ({}),
    reconcileWaitingRoom: () => {},
    setWaitingRoomState: () => {},
    rebuildTranscript: () => {},
    applyLifecycleAction: () => {},
    focusPrompt: () => prompt.focus(),
    activateWaitingRoom: () => { activations.push("activate") },
  })
  harness.renderer.keyInput.on("keypress", (event) => { controller.focusPromptOnSlash(event) })
  return { harness, prompt, activations }
}

for (const [name, send] of [
  ["one terminal write", (stdin: NodeJS.EventEmitter, bytes: string) => { stdin.emit("data", Buffer.from(bytes)) }],
  ["one key at a time", (stdin: NodeJS.EventEmitter, bytes: string) => { for (const char of bytes) stdin.emit("data", Buffer.from(char)) }],
] as const) {
  test(`the first slash command typed in a fresh waiting room reaches the prompt (${name})`, async () => {
    const room = await waitingRoom()
    try {
      assert.equal(room.prompt.focused, false)
      send(room.harness.renderer.stdin, "/kernel add chariox://not-a-link")
      assert.equal(room.prompt.plainText, "/kernel add chariox://not-a-link")
      assert.equal(room.prompt.focused, true)
      assert.deepEqual(room.activations, [])
    } finally {
      room.harness.renderer.destroy()
    }
  })
}

test("other keys in a fresh waiting room do not focus the prompt", async () => {
  const room = await waitingRoom()
  try {
    room.harness.renderer.stdin.emit("data", Buffer.from("kernel"))
    assert.equal(room.prompt.focused, false)
    assert.equal(room.prompt.plainText, "")
  } finally {
    room.harness.renderer.destroy()
  }
})
