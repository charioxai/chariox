import assert from "node:assert/strict"
import test from "node:test"

import { createKernelResyncController } from "./kernel-resync-controller.js"

type TestSession = {
  id: string
  active_provider_run_id?: string | null
  hasPromptWork?: boolean
  projectedRunId?: string | null
  refreshKey?: string
}

type TestProviderRun = {
  id: string
}

function createDeferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test("resync catches up, applies the projected session, refreshes panes, and marks connected", async () => {
  let currentSession: TestSession = { id: "session-1", hasPromptWork: true }
  const events: string[] = []
  const controller = createKernelResyncController<TestSession, TestProviderRun>({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => currentSession.id,
    getSessionStateSnapshot: () => currentSession,
    catchUpAttachedSession: async (sessionId, attachmentId, session) => {
      assert.equal(sessionId, "session-1")
      assert.equal(attachmentId, "attachment-1")
      assert.equal(session.id, "session-1")
      events.push("catch-up")
    },
    getSessionState: async () => ({ id: "session-1", hasPromptWork: false, refreshKey: "next" }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => null,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session, providerRun) => ({ ...session, projectedRunId: providerRun?.id ?? null }),
    shouldRefreshAgentPanesForSessionChange: (session) => session.refreshKey === "next",
    sessionHasPromptWork: (session) => Boolean(session.hasPromptWork),
    applySession: (session) => {
      currentSession = session
      events.push("apply-session")
    },
    applyProviderRun: () => {},
    refreshAgentPanes: async (session) => {
      assert.equal(session.refreshKey, "next")
      events.push("refresh-panes")
    },
    clearLocalBusyStateForAuthoritativeIdle: (session) => {
      assert.equal(session.id, "session-1")
      events.push("clear-busy")
    },
    onProviderRunCleared: () => {},
    onProviderRunRefreshed: () => {},
    onResyncStart: (_sessionId, _attachmentId, reason) => {
      assert.equal(reason, "replay_gap")
      events.push("start")
    },
    onResyncComplete: (reason) => {
      assert.equal(reason, "replay_gap")
      events.push("complete")
    },
    onResyncFailed: () => {},
  })

  await controller.resync("replay_gap")

  assert.deepEqual(events, [
    "start",
    "catch-up",
    "apply-session",
    "refresh-panes",
    "clear-busy",
    "complete",
  ])
  assert.equal(controller.isInFlight(), false)
  assert.deepEqual(currentSession, {
    id: "session-1",
    hasPromptWork: false,
    refreshKey: "next",
    projectedRunId: null,
  })
})

test("resync is idle without an attached session and returns the current in-flight operation", async () => {
  const deferred = createDeferred<TestSession>()
  let attached = false
  let sessionFetches = 0
  const controller = createKernelResyncController<TestSession, TestProviderRun>({
    getAttachment: () => attached ? ({ id: "attachment-1" }) : null,
    isAttached: () => attached,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => ({ id: "session-1" }),
    catchUpAttachedSession: async () => {},
    getSessionState: async () => {
      sessionFetches += 1
      return deferred.promise
    },
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => null,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session) => session,
    shouldRefreshAgentPanesForSessionChange: () => false,
    sessionHasPromptWork: () => false,
    applySession: () => {},
    applyProviderRun: () => {},
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onProviderRunCleared: () => {},
    onProviderRunRefreshed: () => {},
    onResyncStart: () => {},
    onResyncComplete: () => {},
    onResyncFailed: () => {},
  })

  await controller.resync("detached")
  attached = true
  const firstResync = controller.resync("transport_resumed")
  const secondResync = controller.resync("duplicate")
  assert.equal(firstResync, secondResync)
  deferred.resolve({ id: "session-1" })
  await firstResync

  assert.equal(sessionFetches, 1)
})

test("resync clears missing provider runs", async () => {
  let providerRun: TestProviderRun | null = { id: "run-1" }
  const cleared: string[] = []
  const controller = createKernelResyncController<TestSession, TestProviderRun>({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => ({ id: "session-1" }),
    catchUpAttachedSession: async () => {},
    getSessionState: async () => ({ id: "session-1", active_provider_run_id: null }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => providerRun,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session) => session,
    shouldRefreshAgentPanesForSessionChange: () => false,
    sessionHasPromptWork: () => false,
    applySession: () => {},
    applyProviderRun: (run) => {
      providerRun = run
    },
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onProviderRunCleared: (run, sessionId, reason) => {
      cleared.push(`${run.id}:${sessionId}:${reason}`)
    },
    onProviderRunRefreshed: () => {},
    onResyncStart: () => {},
    onResyncComplete: () => {},
    onResyncFailed: () => {},
  })

  await controller.resync("transport_resumed")

  assert.deepEqual(cleared, ["run-1:session-1:transport_resumed"])
  assert.equal(providerRun, null)
})

test("resync refreshes changed provider runs and reapplies the current session", async () => {
  let providerRun: TestProviderRun | null = { id: "run-1" }
  let currentSession: TestSession = { id: "session-1" }
  const refreshed: string[] = []
  const controller = createKernelResyncController<TestSession, TestProviderRun>({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => currentSession,
    catchUpAttachedSession: async () => {},
    getSessionState: async () => ({ id: "session-1", active_provider_run_id: "run-2" }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => providerRun,
    tryGetProviderRun: async () => ({ id: "run-2" }),
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session, run) => ({ ...session, projectedRunId: run?.id ?? null }),
    shouldRefreshAgentPanesForSessionChange: () => false,
    sessionHasPromptWork: () => false,
    applySession: (session) => {
      currentSession = session
    },
    applyProviderRun: (run) => {
      providerRun = run
    },
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onProviderRunCleared: () => {},
    onProviderRunRefreshed: (run, sessionId, previousProviderRunId, reason) => {
      refreshed.push(`${run.id}:${sessionId}:${previousProviderRunId}:${reason}`)
    },
    onResyncStart: () => {},
    onResyncComplete: () => {},
    onResyncFailed: () => {},
  })

  await controller.resync("manual")

  assert.deepEqual(refreshed, ["run-2:session-1:run-1:manual"])
  assert.deepEqual(providerRun, { id: "run-2" })
  assert.equal(currentSession.projectedRunId, "run-2")
})

test("resync reports failures and clears the in-flight state", async () => {
  let failure: unknown
  const controller = createKernelResyncController<TestSession, TestProviderRun>({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => ({ id: "session-1" }),
    catchUpAttachedSession: async () => {
      throw new Error("catch-up failed")
    },
    getSessionState: async () => ({ id: "session-1" }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => null,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session) => session,
    shouldRefreshAgentPanesForSessionChange: () => false,
    sessionHasPromptWork: () => false,
    applySession: () => {},
    applyProviderRun: () => {},
    refreshAgentPanes: async () => {},
    clearLocalBusyStateForAuthoritativeIdle: () => {},
    onProviderRunCleared: () => {},
    onProviderRunRefreshed: () => {},
    onResyncStart: () => {},
    onResyncComplete: () => {},
    onResyncFailed: (_reason, error) => {
      failure = error
    },
  })

  await controller.resync("manual")

  assert.match(failure instanceof Error ? failure.message : String(failure), /catch-up failed/)
  assert.equal(controller.isInFlight(), false)
})
