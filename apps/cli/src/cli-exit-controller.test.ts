import assert from "node:assert/strict"
import test from "node:test"

import { createCliExitController } from "./cli-exit-controller.js"

function createDeferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test("requestExit persists prompt state, detaches the attachment, and exits cleanly", async () => {
  let closing = false
  const events: string[] = []
  const controller = createCliExitController({
    isClosing: () => closing,
    setClosing: (value) => {
      closing = value
      events.push(`closing:${value}`)
    },
    getCreatedSession: () => true,
    getConnectedClientCount: () => 2,
    getAttachment: () => ({ id: "attachment-1", session_id: "session-1" }),
    getSessionId: () => "session-1",
    getPromptDraft: () => "draft",
    syncPromptTextSnapshot: () => {
      events.push("sync")
    },
    flushPromptDraftPersist: async () => {
      events.push("flush")
    },
    persistSessionPromptDraft: async (sessionId, promptDraft) => {
      assert.equal(sessionId, "session-1")
      assert.equal(promptDraft, "draft")
      events.push("persist")
    },
    shouldEndSessionOnExit: () => false,
    archiveSession: async () => {
      events.push("archive")
    },
    detachAttachment: async (attachmentId) => {
      assert.equal(attachmentId, "attachment-1")
      events.push("detach")
    },
    getCleanupDecision: () => ({ exit: false, exitCode: 1, message: "failed" }),
    restoreTerminalAndExit: async (exitCode) => {
      assert.equal(exitCode, 0)
      events.push("exit:0")
    },
    onForceExitAfterCleanupFailure: () => {},
    onExitRequested: (createdSession) => {
      assert.equal(createdSession, true)
      events.push("requested")
    },
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: () => {},
    onCleanupCompleted: () => {
      events.push("completed")
    },
  })

  assert.equal(await controller.requestExit(), true)

  assert.deepEqual(events, [
    "closing:true",
    "requested",
    "sync",
    "flush",
    "persist",
    "detach",
    "completed",
    "exit:0",
  ])
  assert.equal(controller.cleanupFailed(), false)
})

test("requestExit archives the session when the exit policy says to end it", async () => {
  let archivedSessionId: string | null = null
  const controller = createCliExitController({
    isClosing: () => false,
    setClosing: () => {},
    getCreatedSession: () => true,
    getConnectedClientCount: () => 1,
    getAttachment: () => ({ id: "attachment-1", session_id: "session-1" }),
    getSessionId: () => "session-1",
    getPromptDraft: () => "",
    syncPromptTextSnapshot: () => {},
    flushPromptDraftPersist: async () => {},
    persistSessionPromptDraft: async () => {},
    shouldEndSessionOnExit: (createdSession, connectedClientCount) => {
      assert.equal(createdSession, true)
      assert.equal(connectedClientCount, 1)
      return true
    },
    archiveSession: async (sessionId) => {
      archivedSessionId = sessionId
    },
    detachAttachment: async () => {
      throw new Error("unexpected detach")
    },
    getCleanupDecision: () => ({ exit: false, exitCode: 1, message: "failed" }),
    restoreTerminalAndExit: async () => {},
    onForceExitAfterCleanupFailure: () => {},
    onExitRequested: () => {},
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: () => {},
    onCleanupCompleted: () => {},
  })

  await controller.requestExit()

  assert.equal(archivedSessionId, "session-1")
})

test("requestExit logs draft persistence failures but continues cleanup", async () => {
  const failures: string[] = []
  let detached = false
  const controller = createCliExitController({
    isClosing: () => false,
    setClosing: () => {},
    getCreatedSession: () => false,
    getConnectedClientCount: () => 1,
    getAttachment: () => ({ id: "attachment-1", session_id: "session-1" }),
    getSessionId: () => "session-1",
    getPromptDraft: () => "draft",
    syncPromptTextSnapshot: () => {},
    flushPromptDraftPersist: async () => {
      throw new Error("flush failed")
    },
    persistSessionPromptDraft: async () => {
      throw new Error("persist failed")
    },
    shouldEndSessionOnExit: () => false,
    archiveSession: async () => {},
    detachAttachment: async () => {
      detached = true
    },
    getCleanupDecision: () => ({ exit: false, exitCode: 1, message: "failed" }),
    restoreTerminalAndExit: async () => {},
    onForceExitAfterCleanupFailure: () => {},
    onExitRequested: () => {},
    onPromptDraftFlushFailed: (error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
    onPromptDraftPersistFailed: (_sessionId, error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
    onCleanupFailed: () => {},
    onCleanupCompleted: () => {},
  })

  await controller.requestExit()

  assert.deepEqual(failures, ["flush failed", "persist failed"])
  assert.equal(detached, true)
})

test("requestExit reports cleanup failure and lets a second exit force quit", async () => {
  let closing = false
  const events: string[] = []
  const controller = createCliExitController({
    isClosing: () => closing,
    setClosing: (value) => {
      closing = value
      events.push(`closing:${value}`)
    },
    getCreatedSession: () => false,
    getConnectedClientCount: () => 1,
    getAttachment: () => ({ id: "attachment-1", session_id: "session-1" }),
    getSessionId: () => "session-1",
    getPromptDraft: () => "",
    syncPromptTextSnapshot: () => {},
    flushPromptDraftPersist: async () => {},
    persistSessionPromptDraft: async () => {},
    shouldEndSessionOnExit: () => false,
    archiveSession: async () => {},
    detachAttachment: async () => {
      throw new Error("detach failed")
    },
    getCleanupDecision: (error, previousCleanupFailure) => {
      assert.equal(previousCleanupFailure, false)
      return {
        exit: false,
        exitCode: 1,
        message: error instanceof Error ? error.message : String(error),
      }
    },
    restoreTerminalAndExit: async (exitCode) => {
      events.push(`exit:${exitCode}`)
    },
    onForceExitAfterCleanupFailure: () => {
      events.push("force")
    },
    onExitRequested: () => {},
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: (decision) => {
      events.push(`failed:${decision.message}`)
    },
    onCleanupCompleted: () => {},
  })

  assert.equal(await controller.requestExit(), true)
  assert.equal(controller.cleanupFailed(), true)
  closing = true
  assert.equal(await controller.requestExit(), true)

  assert.deepEqual(events, [
    "closing:true",
    "closing:false",
    "failed:detach failed",
    "force",
    "exit:1",
  ])
})

test("requestExit ignores duplicate requests while cleanup is already running", async () => {
  let closing = false
  const deferred = createDeferred<void>()
  let detachCount = 0
  const controller = createCliExitController({
    isClosing: () => closing,
    setClosing: (value) => {
      closing = value
    },
    getCreatedSession: () => false,
    getConnectedClientCount: () => 1,
    getAttachment: () => ({ id: "attachment-1" }),
    getSessionId: () => "session-1",
    getPromptDraft: () => "",
    syncPromptTextSnapshot: () => {},
    flushPromptDraftPersist: async () => {},
    persistSessionPromptDraft: async () => {},
    shouldEndSessionOnExit: () => false,
    archiveSession: async () => {},
    detachAttachment: async () => {
      detachCount += 1
      await deferred.promise
    },
    getCleanupDecision: () => ({ exit: false, exitCode: 1, message: "failed" }),
    restoreTerminalAndExit: async () => {},
    onForceExitAfterCleanupFailure: () => {},
    onExitRequested: () => {},
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: () => {},
    onCleanupCompleted: () => {},
  })

  const firstExit = controller.requestExit()
  assert.equal(await controller.requestExit(), false)
  deferred.resolve()
  await firstExit

  assert.equal(detachCount, 1)
})
