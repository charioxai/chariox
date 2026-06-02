import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import type { SessionListEntry } from "./sessions.js"
import { waitingRoomRows } from "./waiting-room-rows.js"
import { createWaitingRoomState } from "./waiting-room-state.js"

test("waiting room rows compose start, session, remote, slice, terminal, and theme sections", () => {
  const catalog = fallbackProviderCatalog()
  const sessions: SessionListEntry[] = [{
    id: "session-1",
    alias: null,
    worktree_id: "/workspace/tree",
    status: "Active",
    created_at_ms: Date.UTC(2026, 0, 1, 9, 0),
    last_used_at_ms: Date.UTC(2026, 0, 1, 10, 0),
  }]
  const state = {
    ...createWaitingRoomState(sessions, catalog, "opencode", "opencode/gpt-5.4", "high"),
    focus: "slice-entry" as const,
    sliceIndex: 0,
  }
  const rows = waitingRoomRows(state, sessions, catalog, {
    relay: { configured: true, connected: true, relay_url: "wss://relay.example" },
    slices: [{
      id: "slice-1",
      name: "linux-dev",
      owner_kernel_id: "kernel-local",
      owner_machine_id: "machine-local",
      backend: "local_docker",
      os: "linux",
      status: "running",
      workspace_mount: null,
      worker_kernel_ref: "slice:slice-1",
      worker_kernel_id: "kernel-slice",
      worker_machine_id: "machine-slice",
      providers: ["codex"],
      display_endpoint: null,
      created_at_ms: 0,
      updated_at_ms: 0,
    }],
    terminals: [{ terminal_id: "terminal-1", terminal_type: "cli", paired_at_ms: 0, revoked: false }],
  })

  assert.equal(rows[0]?.id, "new")
  assert.equal(rows.some((row) => row.id === "session:session-1"), true)
  assert.equal(rows.some((row) => row.id === "relay-header"), true)
  assert.equal(rows.some((row) => row.id === "slices-header"), true)
  assert.equal(rows.some((row) => row.id === "slice:slice-1"), true)
  assert.equal(rows.find((row) => row.id === "slice:slice-1")?.focused, true)
  assert.equal(rows.some((row) => row.id === "terminal:terminal-1"), true)
  assert.equal(rows.at(-1)?.id, "theme")
})

test("waiting room rows expose local provider catalog fallback", () => {
  const catalog = fallbackProviderCatalog({ source: "local_fallback" })
  const state = createWaitingRoomState([], catalog, "opencode", "opencode/gpt-5.4", "high")
  const rows = waitingRoomRows(state, [], catalog)

  assert.equal(rows.find((row) => row.id === "provider")?.value, "OpenCode (local list)")
  assert.equal(rows.find((row) => row.id === "model")?.value, "GPT-5.4 (local list)")
  assert.equal(rows.find((row) => row.id === "effort")?.value, "High (local list)")
})
