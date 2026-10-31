import assert from "node:assert/strict"
import test from "node:test"
import { ScrollBoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import { followWaitingRoomFocus } from "./waiting-room-scroll.js"
import { buildNoSessionRenderable, waitingRoomMenuWidthFor } from "./workspace-renderables.js"

// The waiting room list is taller than the terminal: the arrow keys move the highlight, so the view follows it.
const sessions: SessionListEntry[] = Array.from({ length: 12 }, (_, index) => ({
  id: `session-${index}`, alias: `Session ${index}`, worktree_id: "/workspace/tree", status: "Active",
  created_at_ms: Date.UTC(2026, 0, 1, 9, index), last_used_at_ms: Date.UTC(2026, 0, 1, 10, index),
}))

async function waitingRoomAt(height: number, focus: "new" | "session" | "theme", sessionIndex = 0) {
  const harness = await createTestRenderer({ width: 90, height, useThread: false })
  const scroll = new ScrollBoxRenderable(harness.renderer, { width: 90, height, stickyScroll: false })
  harness.renderer.root.add(scroll)
  const catalog = fallbackProviderCatalog()
  const state = { ...createWaitingRoomState(sessions, catalog, "codex", "gpt-6-luna", "low"), focus, sessionIndex }
  scroll.add(buildNoSessionRenderable(harness.renderer, state, sessions, catalog, {}, undefined, undefined, 90))
  followWaitingRoomFocus(harness.renderer, () => scroll)
  await harness.renderOnce()
  await harness.renderOnce()
  return { harness, scroll, frame: () => harness.captureCharFrame() }
}

test("the highlighted last row stays on screen when the list is taller than the terminal", async () => {
  const room = await waitingRoomAt(14, "theme")
  try {
    assert.ok(room.scroll.scrollHeight > room.scroll.height, "the list is taller than the terminal")
    assert.match(room.frame(), /Theme/, room.frame())
  } finally { room.harness.renderer.destroy() }
})

test("the highlighted session row stays on screen", async () => {
  const room = await waitingRoomAt(14, "session", 0)
  try {
    assert.match(room.frame(), /session-11/, room.frame())
  } finally { room.harness.renderer.destroy() }
})

test("a highlight at the top shows the top of the list", async () => {
  const room = await waitingRoomAt(14, "new")
  try {
    assert.equal(room.scroll.scrollTop, 0)
  } finally { room.harness.renderer.destroy() }
})

test("the view follows the highlight down the list and back to the top", async () => {
  const harness = await createTestRenderer({ width: 90, height: 14, useThread: false })
  const scroll = new ScrollBoxRenderable(harness.renderer, { width: 90, height: 14, stickyScroll: false })
  harness.renderer.root.add(scroll)
  const catalog = fallbackProviderCatalog()
  const show = async (focus: "new" | "model" | "theme") => {
    for (const child of [...scroll.getChildren()]) { scroll.remove(child.id); child.destroyRecursively() }
    const state = { ...createWaitingRoomState(sessions, catalog, "codex", "gpt-6-luna", "low"), focus }
    scroll.add(buildNoSessionRenderable(harness.renderer, state, sessions, catalog, {}, undefined, undefined, 90))
    followWaitingRoomFocus(harness.renderer, () => scroll)
    await harness.renderOnce()
    await harness.renderOnce()
    return harness.captureCharFrame()
  }
  try {
    assert.match(await show("model"), /Model/)
    assert.match(await show("theme"), /Theme/)
    assert.match(await show("model"), /Model/)
    await show("new")
    assert.equal(scroll.scrollTop, 0)
  } finally { harness.renderer.destroy() }
})

// The scrollbar takes the last column: rows drawn to the full terminal width ran under it and were cut.
test("a menu row and the help text fit the terminal next to the scrollbar", async () => {
  const width = 60
  const harness = await createTestRenderer({ width, height: 40, useThread: false })
  const scroll = new ScrollBoxRenderable(harness.renderer, { width, height: 40, stickyScroll: false })
  harness.renderer.root.add(scroll)
  const catalog = fallbackProviderCatalog()
  const state = { ...createWaitingRoomState([], catalog, "codex", "gpt-6-luna", "low"), focus: "new" as const }
  scroll.add(buildNoSessionRenderable(harness.renderer, state, [], catalog, {}, undefined, undefined, waitingRoomMenuWidthFor(width)))
  try {
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    const machines = frame.split("\n").find(line => line.includes("┃    Machines  "))
    assert.ok(machines && /unavailable until relay is configured|\.\.\.\s*█?$/.test(machines.trimEnd()), `the row is shortened, not cut by the scrollbar: ${machines}`)
    assert.match(frame.replace(/\s+/g, " "), /Type \/ for commands, such as \/credential vault unlock\./, "the help text is whole and names the commands that work here")
  } finally { harness.renderer.destroy() }
})
