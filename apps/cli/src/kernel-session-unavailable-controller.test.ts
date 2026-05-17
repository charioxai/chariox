import assert from "node:assert/strict"
import test from "node:test"

import { createKernelSessionUnavailableController } from "./kernel-session-unavailable-controller.js"

type TestSession = {
  id: string
  projected?: boolean
}

type TestAttachment = {
  id: string
}

type TestProviderRun = {
  id: string
}

test("session unavailable recovery reattaches the still-current session", async () => {
  const harness = createHarness({
    providerRun: { id: "run-1" },
  })

  await harness.controller.handle("Session unavailable.")

  assert.deepEqual(harness.calls, [
    "get-session-state:session-1",
    "attach:session-1",
    "apply-attachment:attachment-1",
    "project:session-1:run-1",
    "apply-session:session-1:true",
    "reset-subscription",
    "sync-subscription",
    "refresh-panes:session-1:true",
    "clear-busy:session-1:true",
    "activity:kernel_session_unavailable_recovered",
    "recovered",
  ])
  assert.deepEqual(harness.session, { id: "session-1", projected: true })
})

test("session unavailable recovery transitions when detached", async () => {
  const harness = createHarness({
    attached: false,
  })

  await harness.controller.handle("Session unavailable.")

  assert.deepEqual(harness.calls, [
    "transition:Session unavailable.",
  ])
})

test("session unavailable recovery transitions when state lookup fails", async () => {
  const harness = createHarness({
    getSessionState: async () => {
      throw new Error("missing")
    },
  })

  await harness.controller.handle("Session unavailable.")

  assert.deepEqual(harness.calls, [
    "lookup-failed:session-1:Session unavailable.:missing",
    "transition:Session unavailable.",
  ])
})

test("session unavailable recovery exits when the selected session changes", async () => {
  const harness = createHarness({
    attachToSession: async (sessionId) => {
      harness.calls.push(`attach:${sessionId}`)
      harness.session = { id: "session-2" }
      return { id: "attachment-1" }
    },
  })

  await harness.controller.handle("Session unavailable.")

  assert.deepEqual(harness.calls, [
    "get-session-state:session-1",
    "attach:session-1",
  ])
})

function createHarness(options: {
  attached?: boolean
  providerRun?: TestProviderRun | null
  getSessionState?: (sessionId: string) => Promise<TestSession>
  attachToSession?: (sessionId: string) => Promise<TestAttachment>
} = {}) {
  const calls: string[] = []
  const harness = {
    calls,
    attached: options.attached ?? true,
    session: { id: "session-1" } as TestSession,
    providerRun: options.providerRun ?? null,
    controller: null as ReturnType<
      typeof createKernelSessionUnavailableController<TestSession, TestAttachment, TestProviderRun>
    > | null,
  }
  harness.controller = createKernelSessionUnavailableController<TestSession, TestAttachment, TestProviderRun>({
    isAttached: () => harness.attached,
    getSession: () => harness.session,
    getProviderRun: () => harness.providerRun,
    getSessionState: options.getSessionState ?? (async (sessionId) => {
      calls.push(`get-session-state:${sessionId}`)
      return { id: sessionId }
    }),
    attachToSession: options.attachToSession ?? (async (sessionId) => {
      calls.push(`attach:${sessionId}`)
      return { id: "attachment-1" }
    }),
    applyAttachment: (attachment) => {
      calls.push(`apply-attachment:${attachment.id}`)
    },
    projectSession: (session, providerRun) => {
      calls.push(`project:${session.id}:${providerRun?.id ?? "null"}`)
      return { ...session, projected: true }
    },
    applySession: (session) => {
      calls.push(`apply-session:${session.id}:${session.projected ?? false}`)
      harness.session = session
    },
    resetKernelEventSubscription: () => {
      calls.push("reset-subscription")
    },
    syncKernelEventSubscription: async () => {
      calls.push("sync-subscription")
    },
    refreshAgentPanes: async (session) => {
      calls.push(`refresh-panes:${session.id}:${session.projected ?? false}`)
    },
    clearLocalBusyStateForAuthoritativeIdle: (session) => {
      calls.push(`clear-busy:${session.id}:${session.projected ?? false}`)
    },
    recordDaemonActivity: (activityType) => {
      calls.push(`activity:${activityType}`)
    },
    onRecovered: () => {
      calls.push("recovered")
    },
    onStateLookupFailed: (sessionId, message, error) => {
      const errorMessage = error instanceof Error ? error.message : String(error)
      calls.push(`lookup-failed:${sessionId}:${message}:${errorMessage}`)
    },
    transitionToNoSession: async (message) => {
      calls.push(`transition:${message}`)
    },
  })

  return harness as typeof harness & {
    controller: ReturnType<
      typeof createKernelSessionUnavailableController<TestSession, TestAttachment, TestProviderRun>
    >
  }
}
