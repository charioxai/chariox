import assert from "node:assert/strict"
import test from "node:test"

import { createWaitingRoomTransitionController } from "./waiting-room-transition-controller.js"

test("requestWaitingRoom persists draft state, detaches, and transitions", async () => {
  const events: string[] = []
  const controller = createWaitingRoomTransitionController({
    isClosing: () => false,
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
    transitionToWaitingRoom: (message) => {
      assert.equal(message, "Returned to waiting room.")
      events.push("transition")
    },
    onWaitingRoomRequested: (createdSession) => {
      assert.equal(createdSession, true)
      events.push("requested")
    },
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: () => {},
    onTransitionCompleted: () => {
      events.push("completed")
    },
  })

  assert.equal(await controller.requestWaitingRoom(), true)

  assert.deepEqual(events, [
    "requested",
    "sync",
    "flush",
    "persist",
    "detach",
    "transition",
    "completed",
  ])
})

test("requestWaitingRoom archives the session when the exit policy says to end it", async () => {
  let archivedSessionId: string | null = null
  const controller = createWaitingRoomTransitionController({
    isClosing: () => false,
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
    transitionToWaitingRoom: () => {},
    onWaitingRoomRequested: () => {},
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: () => {},
    onTransitionCompleted: () => {},
  })

  await controller.requestWaitingRoom()

  assert.equal(archivedSessionId, "session-1")
})

test("requestWaitingRoom logs draft persistence failures but continues cleanup", async () => {
  const failures: string[] = []
  let detached = false
  const controller = createWaitingRoomTransitionController({
    isClosing: () => false,
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
    transitionToWaitingRoom: () => {},
    onWaitingRoomRequested: () => {},
    onPromptDraftFlushFailed: (error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
    onPromptDraftPersistFailed: (_sessionId, error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
    onCleanupFailed: () => {},
    onTransitionCompleted: () => {},
  })

  await controller.requestWaitingRoom()

  assert.deepEqual(failures, ["flush failed", "persist failed"])
  assert.equal(detached, true)
})

test("requestWaitingRoom still transitions when detach cleanup fails", async () => {
  let cleanupFailure: string | null = null
  let transitioned = false
  const controller = createWaitingRoomTransitionController({
    isClosing: () => false,
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
      throw new Error("detach failed")
    },
    transitionToWaitingRoom: () => {
      transitioned = true
    },
    onWaitingRoomRequested: () => {},
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: (error) => {
      cleanupFailure = error instanceof Error ? error.message : String(error)
    },
    onTransitionCompleted: () => {},
  })

  await controller.requestWaitingRoom()

  assert.equal(cleanupFailure, "detach failed")
  assert.equal(transitioned, true)
})

test("requestWaitingRoom is idle while the CLI is closing", async () => {
  let transitioned = false
  const controller = createWaitingRoomTransitionController({
    isClosing: () => true,
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
    detachAttachment: async () => {},
    transitionToWaitingRoom: () => {
      transitioned = true
    },
    onWaitingRoomRequested: () => {},
    onPromptDraftFlushFailed: () => {},
    onPromptDraftPersistFailed: () => {},
    onCleanupFailed: () => {},
    onTransitionCompleted: () => {},
  })

  assert.equal(await controller.requestWaitingRoom(), false)
  assert.equal(transitioned, false)
})
