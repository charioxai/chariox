import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeAttachment, RuntimeSession } from "./cli-types.js"
import { createSessionAttachmentController } from "./session-attachment-controller.js"

function createSession(id: string): RuntimeSession {
  return {
    id,
    project_id: "project-default",
    alias: null,
    workspace_id: "/tmp/workspace",
    worktree_id: "/tmp/workspace",
    created_at_ms: 0,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    config_state: { version: 0, values: {} },
  }
}

test("hydrateCurrentAttachedSession applies the latest session and refreshes panes when the binding is stable", async () => {
  const events: string[] = []
  let session = createSession("session-1")
  const attachment: RuntimeAttachment = { id: "att-1", session_id: "session-1" }
  const latestSession = createSession("session-1")
  latestSession.alias = "latest"

  const controller = createSessionAttachmentController({
    isAttached: () => true,
    attachmentState: () => attachment,
    sessionState: () => session,
    getSessionState: async () => {
      events.push("getSessionState")
      return latestSession
    },
    applySessionState: (nextSession) => {
      events.push("applySessionState")
      session = nextSession
    },
    refreshAgentPanes: async (nextSession) => {
      events.push(`refreshAgentPanes:${nextSession.alias}`)
    },
    refreshSplitPaneFocusRepaint: () => {
      events.push("refreshSplitPaneFocusRepaint")
    },
  })

  const hydrated = await controller.hydrateCurrentAttachedSession("mount")

  assert.equal(hydrated, latestSession)
  assert.equal(session, latestSession)
  assert.deepEqual(events, [
    "getSessionState",
    "applySessionState",
    "refreshAgentPanes:latest",
    "refreshSplitPaneFocusRepaint",
  ])
})

test("hydrateCurrentAttachedSession aborts when the binding changes before hydration completes", async () => {
  const events: string[] = []
  let session = createSession("session-1")
  let attachment: RuntimeAttachment | null = { id: "att-1", session_id: "session-1" }

  const controller = createSessionAttachmentController({
    isAttached: () => attachment !== null,
    attachmentState: () => attachment,
    sessionState: () => session,
    getSessionState: async () => {
      attachment = { id: "att-2", session_id: "session-2" }
      session = createSession("session-2")
      return createSession("session-1")
    },
    applySessionState: () => {
      events.push("applySessionState")
    },
    refreshAgentPanes: async () => {
      events.push("refreshAgentPanes")
    },
  })

  const hydrated = await controller.hydrateCurrentAttachedSession("mount")

  assert.equal(hydrated, null)
  assert.deepEqual(events, [])
})

test("hydrateCurrentAttachedSession falls back to refreshing current panes when hydration fails", async () => {
  const warnings: string[] = []
  const events: string[] = []
  const session = createSession("session-1")
  const attachment: RuntimeAttachment = { id: "att-1", session_id: "session-1" }

  const controller = createSessionAttachmentController({
    isAttached: () => true,
    attachmentState: () => attachment,
    sessionState: () => session,
    getSessionState: async () => {
      throw new Error("network down")
    },
    applySessionState: () => {
      events.push("applySessionState")
    },
    refreshAgentPanes: async (nextSession) => {
      events.push(`refreshAgentPanes:${nextSession.id}`)
    },
    refreshSplitPaneFocusRepaint: () => {
      events.push("refreshSplitPaneFocusRepaint")
    },
    logWarning: (message) => warnings.push(message),
  })

  const hydrated = await controller.hydrateCurrentAttachedSession("mount")

  assert.equal(hydrated, null)
  assert.deepEqual(events, ["refreshAgentPanes:session-1", "refreshSplitPaneFocusRepaint"])
  assert.deepEqual(warnings, ["failed to hydrate attached session state"])
})

test("finalizeAttachedSessionBinding runs resize, catch-up, reload, and pane refresh in order", async () => {
  const events: string[] = []
  const attachedSession = createSession("session-1")
  const hydratedSession = createSession("session-1")
  hydratedSession.alias = "hydrated"

  const controller = createSessionAttachmentController({
    isAttached: () => false,
    attachmentState: () => null,
    sessionState: () => attachedSession,
    getSessionState: async () => {
      events.push("getSessionState")
      return hydratedSession
    },
    applySessionState: () => {
      events.push("applySessionState")
    },
    refreshAgentPanes: async (session) => {
      events.push(`refreshAgentPanes:${session.alias}`)
    },
    maybeResize: async () => {
      events.push("maybeResize")
    },
    catchUpAttachedSession: async () => {
      events.push("catchUpAttachedSession")
    },
  })

  const finalized = await controller.finalizeAttachedSessionBinding({
    sessionId: "session-1",
    attachmentId: "att-1",
    session: attachedSession,
  })

  assert.equal(finalized, hydratedSession)
  assert.deepEqual(events, [
    "maybeResize",
    "catchUpAttachedSession",
    "getSessionState",
    "refreshAgentPanes:hydrated",
  ])
})
