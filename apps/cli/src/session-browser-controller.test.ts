import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { createSessionBrowserController, type SessionBrowserControllerDeps } from "./session-browser-controller.js"
import { clampSessionBrowserIndex, sessionBrowserVisibleSessions } from "@chariox/kernel-client/session-browser-policy"
import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("session browser controller moves selection and requests a rerender", () => {
  const harness = createHarness({ selectedIndex: 0 })
  const controller = createSessionBrowserController(harness.deps)

  assert.equal(controller.handleKey({ name: "down" }), true)
  assert.equal(harness.selectedIndex(), 1)
  assert.equal(harness.renderCount(), 1)

  assert.equal(controller.handleKey({ name: "up" }), true)
  assert.equal(harness.selectedIndex(), 0)
  assert.equal(harness.renderCount(), 2)
})

test("session browser controller closes and attaches selected sessions", async () => {
  const harness = createHarness({ selectedIndex: 0 })
  const controller = createSessionBrowserController(harness.deps)

  assert.equal(controller.handleKey({ name: "enter" }), true)
  await flushMicrotasks()

  assert.equal(harness.isOpen(), false)
  assert.equal(harness.attachedSessions().at(0)?.session.id, "session-b")
  assert.equal(harness.footerMessages().at(-1)?.message, "attached to session Beta")
})

test("session browser controller applies lifecycle actions and clamps remaining selection", async () => {
  const harness = createHarness({
    selectedIndex: 1,
    applyLifecycleAction: async () => {
      harness.setSessions([session("session-a")])
    },
  })
  const controller = createSessionBrowserController(harness.deps)

  assert.equal(controller.handleKey({ name: "delete" }), true)
  await flushMicrotasks()

  assert.equal(harness.lifecycleActions().at(0)?.action, "delete")
  assert.equal(harness.selectedIndex(), 0)
  assert.equal(harness.isOpen(), true)
  assert.equal(harness.renderCount(), 1)
})

test("session browser controller closes after lifecycle removes all visible sessions", async () => {
  const harness = createHarness({
    applyLifecycleAction: async () => {
      harness.setSessions([])
    },
  })
  const controller = createSessionBrowserController(harness.deps)

  assert.equal(controller.handleKey({ name: "a" }), true)
  await flushMicrotasks()

  assert.equal(harness.lifecycleActions().at(0)?.action, "archive")
  assert.equal(harness.isOpen(), false)
})

test("session browser controller keeps archived project sessions inspection-only", async () => {
  const harness = createHarness({ archivedProject: true })
  const controller = createSessionBrowserController(harness.deps)

  assert.equal(controller.handleKey({ name: "enter" }), true)
  assert.equal(controller.handleKey({ name: "delete" }), true)
  await flushMicrotasks()

  assert.equal(harness.isOpen(), true)
  assert.deepEqual(harness.attachedSessions(), [])
  assert.deepEqual(harness.lifecycleActions(), [])
  assert.deepEqual(harness.footerMessages().map((entry) => entry.message), [
    "restore project Archived before opening its sessions",
    "restore project Archived before changing its sessions",
  ])
})

function createHarness(options: {
  selectedIndex?: number
  applyLifecycleAction?: SessionBrowserControllerDeps["applyLifecycleAction"]
  archivedProject?: boolean
} = {}) {
  const catalog = fallbackProviderCatalog()
  let sessions = options.archivedProject
    ? [session("session-ended", { alias: "Ended", status: "Ended", created_at_ms: 3 })]
    : [
      session("session-a", { alias: "Alpha", created_at_ms: 1 }),
      session("session-b", { alias: "Beta", created_at_ms: 2 }),
    ]
  let open = true
  let selectedIndex = options.selectedIndex ?? 0
  let renderCount = 0
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const attachedSessions: Array<{ session: SessionListEntry; createNew: boolean }> = []
  const lifecycleActions: Array<{ action: string; state: WaitingRoomState }> = []

  const deps: SessionBrowserControllerDeps = {
    isOpen: () => open,
    visibleSessions: () => sessionBrowserVisibleSessions(sessions, {
      includeEnded: options.archivedProject === true,
    }),
    availableSessions: () => sessions,
    selectedProject: () => options.archivedProject
      ? {
        id: "project-archived",
        owner_user_id: "owner",
        workspace_id: "/workspace",
        name: "Archived",
        kind: "named",
        status: "archived",
        created_at_ms: 1,
        updated_at_ms: 2,
        archived_at_ms: 2,
        session_count: sessions.length,
        joined_collaborator_count: 0,
        pending_collaboration_invite_count: 0,
      }
      : null,
    normalizeSelectedIndex: () => {
      selectedIndex = clampSessionBrowserIndex(selectedIndex, sessions.length)
      return selectedIndex
    },
    setSelectedIndex: (updater) => {
      selectedIndex = updater(selectedIndex)
    },
    waitingRoomState: () => ({
      ...createWaitingRoomState(sessions, catalog, "opencode", "opencode/gpt-5.4", "medium"),
      focus: "session",
      sessionIndex: selectedIndex,
    }),
    providerCatalog: () => catalog,
    currentProvider: () => "opencode",
    currentModel: () => "opencode/gpt-5.4",
    closeDialog: () => {
      open = false
    },
    renderOverlay: () => {
      renderCount += 1
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    attachSession: async (selected, createNew) => {
      attachedSessions.push({ session: selected, createNew })
    },
    applyLifecycleAction: async (action, state) => {
      lifecycleActions.push({ action, state })
      await options.applyLifecycleAction?.(action, state)
    },
  }

  return {
    deps,
    selectedIndex: () => selectedIndex,
    isOpen: () => open,
    renderCount: () => renderCount,
    footerMessages: () => footerMessages,
    attachedSessions: () => attachedSessions,
    lifecycleActions: () => lifecycleActions,
    setSessions: (nextSessions: SessionListEntry[]) => {
      sessions = nextSessions
    },
  }
}

function session(id: string, overrides: Partial<SessionListEntry> = {}): SessionListEntry {
  return {
    id,
    alias: null,
    worktree_id: "/workspace",
    status: "Created",
    created_at_ms: 1,
    attachment_ids: [],
    ...overrides,
  }
}

async function flushMicrotasks() {
  await Promise.resolve()
  await Promise.resolve()
}
