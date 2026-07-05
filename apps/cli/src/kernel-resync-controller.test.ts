import assert from "node:assert/strict"
import test from "node:test"

import type {
  PromptQueueItem,
  RuntimeProviderRun,
  RuntimeSession,
} from "./cli-types.js"
import { createKernelResyncController } from "./kernel-resync-controller.js"
import { makeSession } from "./command-actions-test-support.js"

function activePrompt(): PromptQueueItem {
  return {
    id: "prompt-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "run",
    status: "Running",
  }
}

function makeProviderRun(id = "run-1"): RuntimeProviderRun {
  return {
    id,
    session_id: "session-1",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    provider: "codex",
    account_profile: "default",
    model: "gpt-5",
    variant: null,
    usage_tokens_total: null,
    state: "Running",
  }
}

function createDeferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test("resync catches up, applies the projected session, refreshes panes, and marks connected", async () => {
  let currentSession: RuntimeSession = makeSession({ active_prompt: activePrompt() })
  const events: string[] = []
  const controller = createKernelResyncController({
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
    getSessionState: async () => makeSession({ alias: "next" }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => null,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session, providerRun) => ({ ...session, workspace_label: providerRun?.id ?? null }),
    shouldRefreshAgentPanesForSessionChange: (session) => session.alias === "next",
    applySession: (session) => {
      currentSession = session
      events.push("apply-session")
    },
    applyProviderRun: () => {},
    refreshAgentPanes: async (session) => {
      assert.equal(session.alias, "next")
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
  assert.equal(currentSession.alias, "next")
  assert.equal(currentSession.workspace_label, null)
  assert.equal(currentSession.active_prompt, null)
})

test("resync is idle without an attached session and returns the current in-flight operation", async () => {
  const deferred = createDeferred<RuntimeSession>()
  let attached = false
  let sessionFetches = 0
  const controller = createKernelResyncController({
    getAttachment: () => attached ? ({ id: "attachment-1" }) : null,
    isAttached: () => attached,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => makeSession(),
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
  deferred.resolve(makeSession())
  await firstResync

  assert.equal(sessionFetches, 1)
})

test("resync clears missing provider runs", async () => {
  let providerRun: RuntimeProviderRun | null = makeProviderRun("run-1")
  const cleared: string[] = []
  const controller = createKernelResyncController({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => makeSession(),
    catchUpAttachedSession: async () => {},
    getSessionState: async () => makeSession({ active_provider_run_id: null }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => providerRun,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session) => session,
    shouldRefreshAgentPanesForSessionChange: () => false,
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
  let providerRun: RuntimeProviderRun | null = makeProviderRun("run-1")
  let currentSession: RuntimeSession = makeSession()
  const refreshed: string[] = []
  const controller = createKernelResyncController({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => currentSession,
    catchUpAttachedSession: async () => {},
    getSessionState: async () => makeSession({ active_provider_run_id: "run-2" }),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => providerRun,
    tryGetProviderRun: async () => makeProviderRun("run-2"),
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session, run) => ({ ...session, workspace_label: run?.id ?? null }),
    shouldRefreshAgentPanesForSessionChange: () => false,
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
  assert.equal(providerRun?.id, "run-2")
  assert.equal(currentSession.workspace_label, "run-2")
})

test("resync reports failures and clears the in-flight state", async () => {
  let failure: unknown
  const controller = createKernelResyncController({
    getAttachment: () => ({ id: "attachment-1" }),
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => makeSession(),
    catchUpAttachedSession: async () => {
      throw new Error("catch-up failed")
    },
    getSessionState: async () => makeSession(),
    getActiveProviderRunId: (session) => session.active_provider_run_id ?? null,
    getProviderRunState: () => null,
    tryGetProviderRun: async () => null,
    sameProviderRun: (currentRun, nextRun) => currentRun.id === nextRun.id,
    projectSession: (session) => session,
    shouldRefreshAgentPanesForSessionChange: () => false,
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
