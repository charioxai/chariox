import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import {
  MAX_VISIBLE_WAITING_ROOM_SESSIONS,
  arrobaArtFrame,
  createWaitingRoomState,
  cycleWaitingRoomValue,
  moveWaitingRoomFocus,
  waitingRoomRows,
} from "./waiting-room.js"

test("waiting room cycles model and effort from provider catalog", () => {
  const catalog = {
    ...fallbackProviderCatalog(),
    all: [
      {
        id: "codex",
        name: "Codex",
        models: {
          "gpt-5.4": { id: "gpt-5.4", name: "GPT-5.4", status: "active", variants: { medium: {}, high: {} } },
        },
      },
      {
        id: "openai",
        name: "OpenAI",
        models: {
          "gpt-5.4": { id: "gpt-5.4", name: "GPT-5.4", status: "active", variants: { low: {}, high: {} } },
          "gpt-5-mini": { id: "gpt-5-mini", name: "GPT-5 mini", status: "active", variants: { low: {} } },
        },
      },
    ],
    default: { codex: "gpt-5.4", openai: "gpt-5.4" },
    connected: ["codex", "openai"],
  }
  let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")
  state = moveWaitingRoomFocus(state, [], 1)
  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "provider")?.value, "Codex")
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "model")?.value, "GPT-5.4")
  state = cycleWaitingRoomValue(state, [], catalog, -1)
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "provider")?.value, "OpenCode")
  state = moveWaitingRoomFocus(state, [], 1)
  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "model")?.value, "GPT-5 mini")
  state = moveWaitingRoomFocus(state, [], 1)
  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "effort")?.value, "Low")
})

test("waiting room renders indented sections and scrolls existing sessions", () => {
  const catalog = fallbackProviderCatalog()
  const sessions = Array.from({ length: MAX_VISIBLE_WAITING_ROOM_SESSIONS + 3 }, (_, index) => ({
    id: `session-${index + 1}`,
    alias: index === 0 ? "alpha" : null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: index % 2 === 0 ? "Active" : "Parked",
    created_at_ms: index,
    attachment_ids: [],
  }))

  let state = createWaitingRoomState(sessions, catalog, "opencode", "openai/gpt-5.4", "high")
  state = moveWaitingRoomFocus(state, sessions, 4)

  const firstWindow = waitingRoomRows(state, sessions, catalog)
  assert.equal(firstWindow[1]?.indent, 1)
  assert.equal(firstWindow[3]?.title, "Variant")
  assert.equal(firstWindow[5]?.title, "alpha")
  assert.equal(firstWindow.filter((row) => row.id.startsWith("session:")).length, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
  assert.equal(firstWindow.some((row) => row.scrollbar === "#"), true)

  for (let step = 0; step < MAX_VISIBLE_WAITING_ROOM_SESSIONS; step += 1) {
    state = moveWaitingRoomFocus(state, sessions, 1)
  }

  const scrolledWindow = waitingRoomRows(state, sessions, catalog)
  assert.equal(scrolledWindow.find((row) => row.focused)?.title, "session-11")
  assert.equal(scrolledWindow[5]?.title, "session-2")
})

test("arrobaArtFrame resolves to the clean logo after the intro completes", () => {
  const first = arrobaArtFrame(0)
  const last = arrobaArtFrame(12)
  assert.notEqual(first, last)
  assert.equal(last.includes("____"), true)
})
