import assert from "node:assert/strict"
import test from "node:test"
import path from "node:path"

import {
  createPromptContentChangeController,
} from "./prompt-content-change-controller.js"

test("handleChange applies text edits, resets active history, and persists the draft", () => {
  let promptText: string | null = "next draft"
  let snapshot = "old draft"
  let historyResetDraft: string | null = null
  const calls: string[] = []
  const persistedDrafts: unknown[] = []
  const controller = createPromptContentChangeController({
    getPromptText: () => promptText,
    isAttached: () => true,
    getPreviousSnapshot: () => snapshot,
    isProgrammaticMutation: () => false,
    isPromptHistoryActive: () => true,
    getSessionId: () => "session-1",
    getCwd: () => process.cwd(),
    setPromptTextSnapshot: (text) => {
      snapshot = text
      calls.push(`snapshot:${text}`)
    },
    resetPromptHistory: (draft) => {
      historyResetDraft = draft
      calls.push(`reset:${draft}`)
    },
    syncPendingAttachmentsFromText: (text) => calls.push(`sync-attachments:${text}`),
    setPromptText: (text) => {
      promptText = text
      calls.push(`prompt:${text}`)
    },
    syncCommandCenter: (text) => calls.push(`command:${text}`),
    schedulePromptDraftPersist: (sessionId, draft) => {
      persistedDrafts.push({ sessionId, draft })
    },
    attachPromptFiles: async () => {},
    onDropFailed: () => {},
  })

  assert.equal(controller.handleChange(), true)

  assert.equal(snapshot, "next draft")
  assert.equal(historyResetDraft, "next draft")
  assert.deepEqual(calls, [
    "reset:next draft",
    "sync-attachments:next draft",
    "snapshot:next draft",
    "command:next draft",
  ])
  assert.deepEqual(persistedDrafts, [{ sessionId: "session-1", draft: "next draft" }])
})

test("handleChange records detached edits without draft persistence", () => {
  let snapshot = ""
  const calls: string[] = []
  const persistedDrafts: unknown[] = []
  const controller = createPromptContentChangeController({
    getPromptText: () => "/session new",
    isAttached: () => false,
    getPreviousSnapshot: () => snapshot,
    isProgrammaticMutation: () => false,
    isPromptHistoryActive: () => false,
    getSessionId: () => null,
    getCwd: () => process.cwd(),
    setPromptTextSnapshot: (text) => {
      snapshot = text
      calls.push(`snapshot:${text}`)
    },
    resetPromptHistory: () => calls.push("reset"),
    syncPendingAttachmentsFromText: () => calls.push("sync-attachments"),
    setPromptText: () => calls.push("prompt"),
    syncCommandCenter: (text) => calls.push(`command:${text}`),
    schedulePromptDraftPersist: (sessionId, draft) => {
      persistedDrafts.push({ sessionId, draft })
    },
    attachPromptFiles: async () => {},
    onDropFailed: () => {},
  })

  assert.equal(controller.handleChange(), true)

  assert.equal(snapshot, "/session new")
  assert.deepEqual(calls, ["snapshot:/session new", "command:/session new"])
  assert.deepEqual(persistedDrafts, [])
})

test("handleChange attaches dropped files and clears pending state after completion", async () => {
  const droppedPath = path.join(process.cwd(), "package.json")
  let promptText: string | null = `attach ${JSON.stringify(droppedPath)}`
  const calls: string[] = []
  const persistedDrafts: unknown[] = []
  const controller = createPromptContentChangeController({
    getPromptText: () => promptText,
    isAttached: () => true,
    getPreviousSnapshot: () => "attach ",
    isProgrammaticMutation: () => false,
    isPromptHistoryActive: () => false,
    getSessionId: () => "session-1",
    getCwd: () => process.cwd(),
    setPromptTextSnapshot: () => calls.push("snapshot"),
    resetPromptHistory: () => calls.push("reset"),
    syncPendingAttachmentsFromText: () => calls.push("sync-attachments"),
    setPromptText: (text) => {
      promptText = text
      calls.push(`prompt:${text}`)
    },
    syncCommandCenter: (text) => calls.push(`command:${text}`),
    schedulePromptDraftPersist: (sessionId, draft) => {
      persistedDrafts.push({ sessionId, draft })
    },
    attachPromptFiles: async (files, insertAt) => {
      calls.push(`attach:${files[0]?.path}:${insertAt}`)
    },
    onDropFailed: () => calls.push("drop-failed"),
  })

  assert.equal(controller.handleChange(), true)
  assert.equal(controller.isDropPending(), true)
  await Promise.resolve()
  await Promise.resolve()

  assert.equal(controller.isDropPending(), false)
  assert.deepEqual(calls, [
    "prompt:attach ",
    "command:attach ",
    `attach:${droppedPath}:7`,
  ])
  assert.deepEqual(persistedDrafts, [{ sessionId: "session-1", draft: "attach " }])
})

test("handleChange reports dropped-file attachment failures", async () => {
  const droppedPath = path.join(process.cwd(), "package.json")
  let failure: unknown
  const controller = createPromptContentChangeController({
    getPromptText: () => `attach ${JSON.stringify(droppedPath)}`,
    isAttached: () => true,
    getPreviousSnapshot: () => "attach ",
    isProgrammaticMutation: () => false,
    isPromptHistoryActive: () => false,
    getSessionId: () => "session-1",
    getCwd: () => process.cwd(),
    setPromptTextSnapshot: () => {},
    resetPromptHistory: () => {},
    syncPendingAttachmentsFromText: () => {},
    setPromptText: () => {},
    syncCommandCenter: () => {},
    schedulePromptDraftPersist: () => {},
    attachPromptFiles: async () => {
      throw new Error("attach failed")
    },
    onDropFailed: (error) => {
      failure = error
    },
  })

  assert.equal(controller.handleChange(), true)
  await Promise.resolve()
  await Promise.resolve()

  assert.match(failure instanceof Error ? failure.message : String(failure), /attach failed/)
  assert.equal(controller.isDropPending(), false)
})

test("handleChange is idle without a prompt input", () => {
  let called = false
  const controller = createPromptContentChangeController({
    getPromptText: () => null,
    isAttached: () => true,
    getPreviousSnapshot: () => "",
    isProgrammaticMutation: () => false,
    isPromptHistoryActive: () => false,
    getSessionId: () => null,
    getCwd: () => process.cwd(),
    setPromptTextSnapshot: () => {
      called = true
    },
    resetPromptHistory: () => {},
    syncPendingAttachmentsFromText: () => {},
    setPromptText: () => {},
    syncCommandCenter: () => {},
    schedulePromptDraftPersist: () => {},
    attachPromptFiles: async () => {},
    onDropFailed: () => {},
  })

  assert.equal(controller.handleChange(), false)
  assert.equal(called, false)
})
