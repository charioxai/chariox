import assert from "node:assert/strict"
import test from "node:test"

import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import {
  createPromptSubmissionUiController,
} from "./prompt-submission-ui-controller.js"

const attachment = (id: string): PendingPromptAttachment => ({
  id,
  url: `file:///tmp/${id}.txt`,
  mime: "text/plain",
  filename: `${id}.txt`,
  kind: "text",
  token: `[file ${id}]`,
})

test("begin snapshots prompt state and clears submitted prompt UI", () => {
  let sessionId: string | null = "session-1"
  let pendingAttachments = [attachment("one")]
  const calls: string[] = []
  const scheduledDrafts: unknown[] = []
  const controller = createPromptSubmissionUiController({
    getSessionId: () => sessionId,
    getPendingAttachments: () => pendingAttachments,
    resetPromptHistoryNavigation: () => calls.push("reset-history"),
    clearDraftPersistQueue: () => calls.push("clear-draft-queue"),
    clearPromptText: () => calls.push("clear-prompt"),
    setPromptText: (text) => calls.push(`set-prompt:${text}`),
    syncPromptTextSnapshot: () => calls.push("sync-snapshot"),
    clearPendingAttachments: () => {
      calls.push("clear-attachments")
      pendingAttachments = []
    },
    setPendingAttachments: (attachments) => {
      pendingAttachments = attachments
    },
    refreshAttachmentHighlights: () => calls.push("refresh-highlights"),
    syncCommandCenter: (text) => calls.push(`sync-command:${text}`),
    retainPromptFocus: () => calls.push("retain-focus"),
    clearCommandCenter: () => calls.push("clear-command"),
    schedulePromptDraftPersist: (nextSessionId, draft) => {
      scheduledDrafts.push({ sessionId: nextSessionId, draft })
    },
    updateSessionChrome: () => calls.push("update-chrome"),
  })

  const snapshot = controller.begin("hello")
  pendingAttachments.push(attachment("two"))
  sessionId = null

  assert.deepEqual(snapshot, {
    rawPrompt: "hello",
    attachments: [attachment("one")],
    sessionId: "session-1",
  })
  assert.notEqual(snapshot.attachments, pendingAttachments)
  assert.deepEqual(calls, [
    "reset-history",
    "clear-draft-queue",
    "clear-prompt",
    "sync-snapshot",
    "clear-attachments",
    "sync-command:",
    "retain-focus",
    "clear-command",
  ])
  assert.deepEqual(scheduledDrafts, [{ sessionId: "session-1", draft: "" }])
})

test("begin skips draft persistence when detached", () => {
  const scheduledDrafts: unknown[] = []
  const controller = createPromptSubmissionUiController({
    getSessionId: () => null,
    getPendingAttachments: () => [],
    resetPromptHistoryNavigation: () => {},
    clearDraftPersistQueue: () => {},
    clearPromptText: () => {},
    setPromptText: () => {},
    syncPromptTextSnapshot: () => {},
    clearPendingAttachments: () => {},
    setPendingAttachments: () => {},
    refreshAttachmentHighlights: () => {},
    syncCommandCenter: () => {},
    retainPromptFocus: () => {},
    clearCommandCenter: () => {},
    schedulePromptDraftPersist: (sessionId, draft) => {
      scheduledDrafts.push({ sessionId, draft })
    },
    updateSessionChrome: () => {},
  })

  assert.deepEqual(controller.begin("detached"), {
    rawPrompt: "detached",
    attachments: [],
    sessionId: null,
  })
  assert.deepEqual(scheduledDrafts, [])
})

test("restore reapplies failed prompt UI and persists the restored draft", () => {
  let pendingAttachments: PendingPromptAttachment[] = []
  const calls: string[] = []
  const scheduledDrafts: unknown[] = []
  const controller = createPromptSubmissionUiController({
    getSessionId: () => null,
    getPendingAttachments: () => [],
    resetPromptHistoryNavigation: () => calls.push("reset-history"),
    clearDraftPersistQueue: () => calls.push("clear-draft-queue"),
    clearPromptText: () => calls.push("clear-prompt"),
    setPromptText: (text) => calls.push(`set-prompt:${text}`),
    syncPromptTextSnapshot: () => calls.push("sync-snapshot"),
    clearPendingAttachments: () => calls.push("clear-attachments"),
    setPendingAttachments: (attachments) => {
      calls.push("set-attachments")
      pendingAttachments = attachments
    },
    refreshAttachmentHighlights: () => calls.push("refresh-highlights"),
    syncCommandCenter: (text) => calls.push(`sync-command:${text}`),
    retainPromptFocus: () => calls.push("retain-focus"),
    clearCommandCenter: () => calls.push("clear-command"),
    schedulePromptDraftPersist: (sessionId, draft) => {
      scheduledDrafts.push({ sessionId, draft })
    },
    updateSessionChrome: () => calls.push("update-chrome"),
  })

  const snapshot = {
    rawPrompt: "restore me",
    attachments: [attachment("one")],
    sessionId: "session-1",
  }

  assert.equal(controller.restore(snapshot), true)
  pendingAttachments.push(attachment("two"))

  assert.deepEqual(calls, [
    "reset-history",
    "set-attachments",
    "set-prompt:restore me",
    "sync-snapshot",
    "refresh-highlights",
    "sync-command:restore me",
    "retain-focus",
    "update-chrome",
  ])
  assert.deepEqual(scheduledDrafts, [{ sessionId: "session-1", draft: "restore me" }])
  assert.deepEqual(snapshot.attachments, [attachment("one")])
})

test("restore is idle without a snapshot", () => {
  let called = false
  const controller = createPromptSubmissionUiController({
    getSessionId: () => null,
    getPendingAttachments: () => [],
    resetPromptHistoryNavigation: () => {
      called = true
    },
    clearDraftPersistQueue: () => {},
    clearPromptText: () => {},
    setPromptText: () => {},
    syncPromptTextSnapshot: () => {},
    clearPendingAttachments: () => {},
    setPendingAttachments: () => {},
    refreshAttachmentHighlights: () => {},
    syncCommandCenter: () => {},
    retainPromptFocus: () => {},
    clearCommandCenter: () => {},
    schedulePromptDraftPersist: () => {},
    updateSessionChrome: () => {},
  })

  assert.equal(controller.restore(null), false)
  assert.equal(called, false)
})
