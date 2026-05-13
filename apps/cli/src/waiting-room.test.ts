import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import {
  MAX_VISIBLE_WAITING_ROOM_SESSIONS,
  arrobaArtFrame,
  createWaitingRoomState,
  cycleWaitingRoomValue,
  moveWaitingRoomFocus,
  waitingRoomChoice,
  waitingRoomRows,
} from "./waiting-room.js"
import { __setWaitingRoomWorktreeInventoryForTest } from "./waiting-room-worktrees.js"

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

  state = moveWaitingRoomFocus(state, [], -1)
  assert.equal(state.focus, "theme")

  state = cycleWaitingRoomValue(state, [], catalog, 1)
  assert.equal(state.themeId, "matrix")
  assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "theme")?.value, "Matrix")
})

test("waiting room cycles existing worktrees and the create-worktree option", () => {
  __setWaitingRoomWorktreeInventoryForTest({
    workspacePath: "/workspace",
    currentWorktreePath: "/workspace",
    options: [
      {
        id: "existing:/workspace",
        kind: "existing",
        label: "main",
        path: "/workspace",
        branch: "main",
        isCurrent: true,
      },
      {
        id: "existing:/workspace-feature",
        kind: "existing",
        label: "feature/login",
        path: "/workspace-feature",
        branch: "feature/login",
        isCurrent: false,
      },
      {
        id: "create-worktree",
        kind: "create",
        label: "Create worktree",
      },
    ],
  })

  try {
    const catalog = fallbackProviderCatalog()
    let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")

    state = moveWaitingRoomFocus(state, [], 5)
    assert.equal(state.focus, "worktree")
    assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "worktree")?.value, "main")

    state = cycleWaitingRoomValue(state, [], catalog, 1)
    assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "worktree")?.value, "feature/login")

    state = cycleWaitingRoomValue(state, [], catalog, 1)
    assert.equal(waitingRoomRows(state, [], catalog).find((row) => row.id === "worktree")?.value, "Create worktree")
  } finally {
    __setWaitingRoomWorktreeInventoryForTest(null)
  }
})

test("waiting room cycles slices for new sessions", () => {
  const catalog = fallbackProviderCatalog()
  const slices = [{
    id: "slice-1",
    name: "linux-dev",
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker" as const,
    os: "linux",
    status: "running" as const,
    workspace_mount: null,
    worker_kernel_ref: "slice:slice-1",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    providers: ["codex"],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }]
  let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")

  state = moveWaitingRoomFocus(state, [], 6, { slices })
  assert.equal(state.focus, "slice")
  assert.equal(waitingRoomRows(state, [], catalog, { slices }).find((row) => row.id === "slice")?.value, "None")

  state = cycleWaitingRoomValue(state, [], catalog, 1, undefined, { slices })
  assert.equal(waitingRoomChoice(state, [], catalog, { slices }).sliceRef, "slice-1")
  assert.equal(waitingRoomRows(state, [], catalog, { slices }).find((row) => row.id === "slice")?.value, "linux-dev")
})

test("waiting room renders indented sections and only previews the last two active sessions", () => {
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
  state = moveWaitingRoomFocus(state, sessions, 6)

  const firstWindow = waitingRoomRows(state, sessions, catalog)
  assert.equal(firstWindow[1]?.id, "provider")
  assert.equal(firstWindow[1]?.indent, 1)
  assert.equal(firstWindow[2]?.id, "model")
  assert.equal(firstWindow[3]?.id, "effort")
  assert.equal(firstWindow[4]?.id, "workspace")
  assert.equal(firstWindow[5]?.id, "worktree")
  assert.equal(firstWindow[6]?.id, "slice")
  assert.equal(firstWindow[6]?.value, "None")
  assert.equal(firstWindow[7]?.id, "join-header")
  assert.equal(firstWindow[7]?.indent, 0)
  assert.equal(firstWindow[7]?.focused, true)
  assert.equal(firstWindow[7]?.value, "Press Enter")
  assert.equal(firstWindow.at(-1)?.id, "theme")
  assert.equal(firstWindow.at(-1)?.indent, 0)
  assert.deepEqual(
    firstWindow.find((row) => row.id === "session-header")?.columns?.map((cell) => cell.trim()),
    ["Status", "Last used", "Created at"],
  )
  const firstSessionRow = firstWindow.find((row) => row.id === "session:session-5")
  assert.equal(firstSessionRow?.title, "session-5")
  assert.deepEqual(
    firstSessionRow?.columns?.map((cell) => cell.trim()),
    ["Active", "2026-04-06 11:04 UTC", "2026-04-06 10:04 UTC"],
  )
  assert.equal(firstWindow.filter((row) => row.id.startsWith("session:")).length, MAX_VISIBLE_WAITING_ROOM_SESSIONS)
  assert.equal(firstWindow.some((row) => row.id === "session:session-3"), false)

  for (let step = 0; step < MAX_VISIBLE_WAITING_ROOM_SESSIONS; step += 1) {
    state = moveWaitingRoomFocus(state, sessions, 1)
  }

  const scrolledWindow = waitingRoomRows(state, sessions, catalog)
  assert.equal(scrolledWindow.find((row) => row.focused)?.title, "session-4")
  assert.deepEqual(scrolledWindow.filter((row) => row.id.startsWith("session:")).map((row) => row.title), [
    "session-5",
    "session-4",
  ])
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

test("waiting room marks sessions with public activity as working", () => {
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
    activity: {
      agent_count: 2,
      working_agent_count: 1,
      active_prompt_count: 1,
      queued_prompt_count: 0,
      error_agent_count: 0,
    },
  }]

  const state = createWaitingRoomState(sessions, catalog, "opencode", "openai/gpt-5.4", "high")
  const rows = waitingRoomRows(state, sessions, catalog)
  const activeSessionRow = rows.find((row) => row.id === "session:session-1")

  assert.equal(activeSessionRow?.title, "* session-1 (frontend)")
  assert.equal(activeSessionRow?.value, "Working")
  assert.equal(activeSessionRow?.columns?.[0]?.trim(), "Working")
})

test("waiting room places join below start configuration and makes cloud relay login selectable", () => {
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
  assert.equal(rows[4]?.id, "workspace")
  assert.equal(rows[5]?.id, "worktree")
  assert.equal(rows[6]?.id, "slice")
  assert.equal(rows[6]?.value, "None")
  assert.equal(rows[7]?.id, "join-header")
  assert.equal(rows[7]?.title, "Join Existing Session")
  assert.equal(rows.at(-1)?.id, "theme")
  assert.equal(rows.at(-1)?.indent, 0)

  state = moveWaitingRoomFocus(state, [], 6)
  assert.equal(state.focus, "relay")
  const relayRows = waitingRoomRows(state, [], catalog, {
    relay: {
      configured: false,
      connected: false,
    },
  })
  const relayConfigure = relayRows.find((row) => row.id === "relay-configure")
  assert.equal(relayConfigure?.title, "Cloud")
  assert.equal(relayConfigure?.value, "/cloud")
  assert.equal(relayConfigure?.selectable, true)
  assert.equal(relayConfigure?.focused, true)

  const cloudNoticeRows = waitingRoomRows(state, [], catalog, {
    cloudNotice: "Opening Arroba Cloud.\nurl=https://cloud.example/terminal?view=waiting",
    relay: {
      configured: false,
      connected: false,
    },
  })
  assert.equal(cloudNoticeRows.find((row) => row.id === "cloud-notice:0")?.title, "Cloud status")
  assert.equal(cloudNoticeRows.find((row) => row.id === "cloud-notice:0")?.value, "Opening Arroba Cloud.")
  assert.equal(cloudNoticeRows.find((row) => row.id === "cloud-notice:1")?.value, "url=https://cloud.example/terminal?view=waiting")
})

test("waiting room shows loading rows before inventory arrives", () => {
  const catalog = fallbackProviderCatalog()
  const state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")
  const rows = waitingRoomRows(state, [], catalog, {
    inventoryStatus: "loading",
    loadingFrame: 2,
  })

  assert.equal(rows.find((row) => row.id === "join-header")?.value, "loading..")
  assert.equal(rows.some((row) => row.id === "no-sessions"), false)
  assert.equal(rows.find((row) => row.id === "sessions-loading")?.value, "loading..")
  assert.equal(rows.find((row) => row.id === "machines-loading")?.value, "loading..")
})

test("waiting room shows relay kernels as selectable targets", () => {
  const catalog = fallbackProviderCatalog()
  let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")
  const remote = {
    relay: {
      configured: true,
      connected: true,
      relay_url: "wss://relay.example",
    },
    machines: [{
      machine_id: "machine-1",
      machine_alias: "builder",
      display_name: "builder",
      trust_status: "approved" as const,
      online: true,
      pending: false,
      kernel_count: 1,
      available_providers: ["opencode"],
    }],
    kernels: [{
      kernel_id: "kernel-1",
      machine_id: "machine-1",
      machine_alias: "builder",
      relay_alias: "builder-kernel",
      available_providers: ["opencode", "codex"],
      accepting_remote_leases: true,
      leased_agent_count: 0,
      local_session_count: 2,
    }],
  }

  state = moveWaitingRoomFocus(state, [], 8, remote)
  assert.equal(state.focus, "remote-kernel")

  const rows = waitingRoomRows(state, [], catalog, remote)
  const kernelRow = rows.find((row) => row.id === "remote-kernel:kernel-1")
  assert.equal(kernelRow?.title, "builder-kernel @ builder")
  assert.equal(kernelRow?.value, "ready opencode,codex")
  assert.equal(kernelRow?.selectable, true)
  assert.equal(kernelRow?.focused, true)
})

test("waiting room makes inactive machines and kernels selectable for deletion", () => {
  const catalog = fallbackProviderCatalog()
  let state = createWaitingRoomState([], catalog, "opencode", "openai/gpt-5.4", "high")
  const remote = {
    relay: {
      configured: true,
      connected: true,
      relay_url: "wss://relay.example",
    },
    machines: [{
      machine_id: "machine-offline",
      machine_alias: "offline-builder",
      display_name: "offline-builder",
      trust_status: "approved" as const,
      online: false,
      pending: false,
      kernel_count: 0,
      available_providers: [],
    }],
    kernels: [{
      kernel_id: "kernel-inactive",
      machine_id: "machine-offline",
      machine_alias: "offline-builder",
      relay_alias: "inactive-kernel",
      available_providers: ["opencode"],
      accepting_remote_leases: false,
      leased_agent_count: 0,
      local_session_count: 0,
    }],
  }

  state = moveWaitingRoomFocus(state, [], 7, remote)
  assert.equal(state.focus, "machine")
  let rows = waitingRoomRows(state, [], catalog, remote)
  const machineRow = rows.find((row) => row.id === "machine:machine-offline")
  assert.equal(machineRow?.selectable, true)
  assert.equal(machineRow?.focused, true)

  state = moveWaitingRoomFocus(state, [], 1, remote)
  assert.equal(state.focus, "remote-kernel")
  rows = waitingRoomRows(state, [], catalog, remote)
  const kernelRow = rows.find((row) => row.id === "remote-kernel:kernel-inactive")
  assert.equal(kernelRow?.value, "inactive opencode")
  assert.equal(kernelRow?.selectable, true)
  assert.equal(kernelRow?.focused, true)
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
  state = moveWaitingRoomFocus(state, sessions, 4)
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
