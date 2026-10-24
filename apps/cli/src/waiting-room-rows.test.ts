import assert from "node:assert/strict"
import test from "node:test"

import { getProviderCatalog } from "./provider-api.js"
import type { LocalIpcClient } from "./ipc.js"
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
  assert.equal(rows.find((row) => row.id === "model")?.value, "gpt-5.4 (local list)")
  assert.equal(rows.find((row) => row.id === "effort")?.value, "High (local list)")
})

// MP-08/MP-10: a completed failed read is not an in-flight catalog.
test("failed catalog plus successful empty inventory leaves retry and kernel selection reachable", async () => {
  const client = { send: async () => { throw new Error("catalog request timed out") } } as unknown as LocalIpcClient
  const catalog = await getProviderCatalog(client)
  assert.equal(catalog.source, "local_fallback")
  assert.ok(catalog.unavailable_reason)
  const state = createWaitingRoomState([], catalog, "codex", "default", "low")
  // Inventory polling can succeed indefinitely without a new catalog event.
  for (let i = 0; i < 3; i++) {
    const rows = waitingRoomRows(state, [], catalog, { inventoryStatus: "ready", collaborationBackend: "cloud",
      relay: { configured: true, connected: true, daemon_id: "kernel-a" },
      kernels: [{ kernel_id: "kernel-a", machine_id: "machine-a" }, { kernel_id: "kernel-b", machine_id: "machine-b" }],
    })
    for (const id of ["new", "launch-machine", "launch-kernel", "provider", "model"]) {
      assert.equal(rows.find(row => row.id === id)?.selectable, true, id)
      assert.doesNotMatch(rows.find(row => row.id === id)!.value, /^loading/, id)
    }
    assert.match(rows.find(row => row.id === "catalog-error")?.value ?? "", /failed.*Provider or Kernel.*retry/i)
  }
})

// MP-08 / MP-10 / MP-11: local inventory need not include itself in remote kernels.
test("ENV-UX-03 current kernel name is visible without a peer directory entry", () => {
  const catalog = fallbackProviderCatalog()
  const state = createWaitingRoomState([], catalog, "codex", "gpt-6-luna", "low")
  const remote = {
    relay: { configured: true, connected: true, daemon_id: "kernel-local", daemon_alias: "My laptop" },
    projects: [{ id: "project", owner_user_id: "owner", workspace_id: "/src", workspace_ids: ["/src"], name: "Source", kind: "default" as const, status: "active" as const, created_at_ms: 1, updated_at_ms: 1, archived_at_ms: null, session_count: 0, last_session_activity_at_ms: null, joined_collaborator_count: 0, pending_collaboration_invite_count: 0 }],
  }
  const rows = waitingRoomRows(state, [], catalog, remote)
  assert.equal(rows.find(row => row.id === "project-kernel:project")?.title, "Kernel: My laptop")
})
