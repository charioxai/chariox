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

test("waiting room cycles selectable themes", () => {
  const catalog = fallbackProviderCatalog()
  let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high", "sober")

  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "theme")?.value, "Sober")

  state = moveWaitingRoomFocus(state, [], 4)
  assert.equal(state.focus, "theme")

  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(state.themeId, "matrix")
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "theme")?.value, "Matrix")
})

test("waiting room renders indented sections and scrolls existing sessions", () => {
  const catalog = fallbackProviderCatalog()
  const baseCreatedAt = Date.UTC(2026, 3, 6, 10, 0)
  const sessions = Array.from({ length: MAX_VISIBLE_WAITING_ROOM_SESSIONS + 3 }, (_, index) => ({
    id: `session-${index + 1}`,
    alias: index === 0 ? "alpha" : null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: index % 2 === 0 ? "Active" : "Parked",
    created_at_ms: baseCreatedAt + index * 60_000,
    last_used_at_ms: baseCreatedAt + index * 60_000 + 3_600_000,
    attachment_ids: [],
  }))

  let state = createWaitingRoomState(sessions, catalog, "opencode", "openai/gpt-5.4", "high")
  state = moveWaitingRoomFocus(state, sessions, 5)

  const firstWindow = waitingRoomRows(state, sessions, catalog)
  assert.equal(firstWindow[1]?.id, "provider")
  assert.equal(firstWindow[1]?.indent, 1)
  assert.equal(firstWindow[2]?.id, "model")
  assert.equal(firstWindow[3]?.id, "effort")
  assert.equal(firstWindow[4]?.id, "theme")
  assert.equal(firstWindow[5]?.id, "join-header")
  assert.equal(firstWindow[5]?.indent, 0)
  assert.deepEqual(
    firstWindow.find((row) => row.id === "session-header")?.columns?.map((cell) => cell.trim()),
    ["Status", "Last used", "Created at"],
  )
  const firstSessionRow = firstWindow.find((row) => row.id === "session:session-1")
  assert.equal(firstSessionRow?.title, "session-1 (alpha)")
  assert.deepEqual(
    firstSessionRow?.columns?.map((cell) => cell.trim()),
    ["Active", "2026-04-06 11:00 UTC", "2026-04-06 10:00 UTC"],
  )
  assert.equal(firstWindow.filter((row) => row.id.startsWith("session:")).length, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
  assert.equal(firstWindow.some((row) => row.scrollbar === "#"), true)

  for (let step = 0; step < MAX_VISIBLE_WAITING_ROOM_SESSIONS; step += 1) {
    state = moveWaitingRoomFocus(state, sessions, 1)
  }

  const scrolledWindow = waitingRoomRows(state, sessions, catalog)
  assert.equal(scrolledWindow.find((row) => row.focused)?.title, "session-11")
  assert.equal(scrolledWindow.find((row) => row.id.startsWith("session:"))?.title, "session-2")
})

test("waiting room renders session rows with alias text", () => {
  const catalog = fallbackProviderCatalog()
  const sessions = [{
    id: "session-1",
    alias: "frontend",
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: "Active",
    created_at_ms: Date.UTC(2026, 3, 6, 10, 0),
    last_used_at_ms: Date.UTC(2026, 3, 6, 11, 0),
    attachment_ids: [],
  }]

  const state = createWaitingRoomState(sessions, catalog, "opencode", "openai/gpt-5.4", "high")
  const rows = waitingRoomRows(state, sessions, catalog)
  const aliasedSessionRow = rows.find((row) => row.id === "session:session-1")

  assert.equal(aliasedSessionRow?.title, "session-1 (frontend)")
})

test("waiting room places join below start configuration and makes relay configure selectable", () => {
  const catalog = fallbackProviderCatalog()
  let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")
  const rows = waitingRoomRows(state, [], catalog, {
    relay: {
      configured: false,
      connected: false,
    },
  })

  assert.equal(rows[0]?.id, "new")
  assert.equal(rows[1]?.id, "provider")
  assert.equal(rows[2]?.id, "model")
  assert.equal(rows[3]?.id, "effort")
  assert.equal(rows[4]?.id, "theme")
  assert.equal(rows[5]?.id, "join-header")
  assert.equal(rows[5]?.title, "Join Existing Session")

  state = moveWaitingRoomFocus(state, [], 5)
  assert.equal(state.focus, "relay")
  const relayRows = waitingRoomRows(state, [], catalog, {
    relay: {
      configured: false,
      connected: false,
    },
  })
  const relayConfigure = relayRows.find((row) => row.id === "relay-configure")
  assert.equal(relayConfigure?.selectable, true)
  assert.equal(relayConfigure?.focused, true)
})

test("waiting room keeps session metadata column widths stable across scroll windows", () => {
  const catalog = fallbackProviderCatalog()
  const sessions = Array.from({ length: 12 }, (_, index) => ({
    id: `session-${index + 1}`,
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: index === 11 ? "Disconnected" : "Active",
    created_at_ms: Date.UTC(2026, 3, 6, 10, 0),
    last_used_at_ms: Date.UTC(2026, 3, 6, 11, 0),
    attachment_ids: [],
  }))

  let state = createWaitingRoomState(sessions, catalog, "opencode", "openai/gpt-5.4", "high")
  state = moveWaitingRoomFocus(state, sessions, 5)
  const firstWindow = waitingRoomRows(state, sessions, catalog)
  const scrolledWindowHeader = firstWindow.find((row) => row.id === "session-header")?.columns
  const firstWindowWidths = scrolledWindowHeader?.map((column) => column.length)
  assert.deepEqual(firstWindowWidths, [12, 20, 20], "windowed rows should use the long status width baseline")

  for (let step = 0; step < MAX_VISIBLE_WAITING_ROOM_SESSIONS; step += 1) {
    state = moveWaitingRoomFocus(state, sessions, 1)
  }

  const secondWindow = waitingRoomRows(state, sessions, catalog)
  const secondWindowHeader = secondWindow.find((row) => row.id === "session-header")?.columns
  const secondWindowWidths = secondWindowHeader?.map((column) => column.length)
  assert.deepEqual(secondWindowWidths, firstWindowWidths)
})

test("arrobaArtFrame resolves to the clean logo after the intro completes", () => {
  const first = arrobaArtFrame(0)
  const last = arrobaArtFrame(12)
  assert.notEqual(first, last)
  assert.equal(last.includes("____"), true)
})
