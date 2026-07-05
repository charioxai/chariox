import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { createSessionBrowserController, type SessionBrowserControllerDeps } from "./session-browser-controller.js"
import { clampSessionBrowserIndex, sessionBrowserVisibleSessions } from "@arroba/kernel-client/session-browser-policy"
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

function createHarness(options: {
  selectedIndex?: number
  applyLifecycleAction?: SessionBrowserControllerDeps["applyLifecycleAction"]
} = {}) {
  const catalog = fallbackProviderCatalog()
  let sessions = [
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
    visibleSessions: () => sessionBrowserVisibleSessions(sessions),
    availableSessions: () => sessions,
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
