import assert from "node:assert/strict"
import test from "node:test"

import { createKernelRestartRecoveryController } from "./kernel-restart-recovery-controller.js"

type TestSession = {
  id: string
  projected?: boolean
}

type TestAttachment = {
  id: string
}

function createDeferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test("recover reattaches the current disconnected session", async () => {
  const events: string[] = []
  const controller = createKernelRestartRecoveryController<TestSession, TestAttachment>({
    isClosing: () => false,
    isAttached: () => true,
    isDisconnected: () => true,
    getSessionId: () => "session-1",
    getSessionState: async (sessionId) => {
      assert.equal(sessionId, "session-1")
      events.push("session")
      return { id: sessionId }
    },
    attachToSession: async (sessionId) => {
      assert.equal(sessionId, "session-1")
      events.push("attach")
      return { id: "attachment-1" }
    },
    projectSession: (session) => {
      events.push("project")
      return { ...session, projected: true }
    },
    applyAttachment: (attachment) => {
      assert.equal(attachment.id, "attachment-1")
      events.push("apply-attachment")
    },
    applySession: (session) => {
      assert.deepEqual(session, { id: "session-1", projected: true })
      events.push("apply-session")
    },
    resetKernelEventSubscription: () => {
      events.push("reset-subscription")
    },
    syncKernelEventSubscription: async () => {
      events.push("sync-subscription")
    },
    refreshAgentPanes: async () => {
      events.push("refresh-panes")
    },
    clearLocalBusyStateForAuthoritativeIdle: () => {
      events.push("clear-busy")
    },
    onRecovered: () => {
      events.push("recovered")
    },
    onAttemptFailed: () => {},
    sleep: async () => {},
  })

  const recovery = controller.recover()
  assert.notEqual(recovery, null)
  assert.equal(controller.isInFlight(), true)
  await recovery

  assert.deepEqual(events, [
    "session",
    "attach",
    "apply-attachment",
    "project",
    "apply-session",
    "reset-subscription",
    "sync-subscription",
    "refresh-panes",
    "clear-busy",
    "recovered",
  ])
  assert.equal(controller.isInFlight(), false)
})

test("recover is idle when detached or no session is selected", () => {
  const controller = createKernelRestartRecoveryController<TestSession, TestAttachment>({
    isClosing: () => false,
    isAttached: () => false,
    isDisconnected: () => true,
    getSessionId: () => "session-1",
    getSessionState: async () => ({ id: "session-1" }),
    attachToSession: async () => ({ id: "attachment-1" }),
    projectSession: (session) => session,
    applyAttachment: () => {},
    applySession: () => {},
    resetKernelEventSubscription: () => {},
    syncKernelEventSubscription: async () => {},
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onRecovered: () => {},
    onAttemptFailed: () => {},
    sleep: async () => {},
  })

  assert.equal(controller.recover(), null)
})

test("recover returns the current in-flight recovery", async () => {
  const deferred = createDeferred<TestSession>()
  let sessionFetches = 0
  const controller = createKernelRestartRecoveryController<TestSession, TestAttachment>({
    isClosing: () => false,
    isAttached: () => true,
    isDisconnected: () => true,
    getSessionId: () => "session-1",
    getSessionState: async () => {
      sessionFetches += 1
      return deferred.promise
    },
    attachToSession: async () => ({ id: "attachment-1" }),
    projectSession: (session) => session,
    applyAttachment: () => {},
    applySession: () => {},
    resetKernelEventSubscription: () => {},
    syncKernelEventSubscription: async () => {},
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onRecovered: () => {},
    onAttemptFailed: () => {},
    sleep: async () => {},
  })

  const firstRecovery = controller.recover()
  const secondRecovery = controller.recover()
  assert.equal(firstRecovery, secondRecovery)
  deferred.resolve({ id: "session-1" })
  await firstRecovery

  assert.equal(sessionFetches, 1)
})

test("recover retries failed attempts with capped backoff", async () => {
  let attempts = 0
  const delays: number[] = []
  const failures: string[] = []
  const controller = createKernelRestartRecoveryController<TestSession, TestAttachment>({
    initialDelayMs: 100,
    maxDelayMs: 150,
    isClosing: () => false,
    isAttached: () => true,
    isDisconnected: () => true,
    getSessionId: () => "session-1",
    getSessionState: async () => {
      attempts += 1
      if (attempts < 3) {
        throw new Error(`failure ${attempts}`)
      }
      return { id: "session-1" }
    },
    attachToSession: async () => ({ id: "attachment-1" }),
    projectSession: (session) => session,
    applyAttachment: () => {},
    applySession: () => {},
    resetKernelEventSubscription: () => {},
    syncKernelEventSubscription: async () => {},
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onRecovered: () => {},
    onAttemptFailed: (_sessionId, error) => {
      failures.push(error instanceof Error ? error.message : String(error))
    },
    sleep: async (delayMs) => {
      delays.push(delayMs)
    },
  })

  await controller.recover()

  assert.deepEqual(failures, ["failure 1", "failure 2"])
  assert.deepEqual(delays, [100, 150])
  assert.equal(attempts, 3)
})

test("recover aborts when the selected session changes during recovery", async () => {
  let currentSessionId = "session-1"
  let attached = false
  const controller = createKernelRestartRecoveryController<TestSession, TestAttachment>({
    isClosing: () => false,
    isAttached: () => true,
    isDisconnected: () => true,
    getSessionId: () => currentSessionId,
    getSessionState: async () => {
      currentSessionId = "session-2"
      return { id: "session-1" }
    },
    attachToSession: async () => {
      attached = true
      return { id: "attachment-1" }
    },
    projectSession: (session) => session,
    applyAttachment: () => {},
    applySession: () => {},
    resetKernelEventSubscription: () => {},
    syncKernelEventSubscription: async () => {},
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onRecovered: () => {},
    onAttemptFailed: () => {},
    sleep: async () => {},
  })

  await controller.recover()

  assert.equal(attached, false)
})
