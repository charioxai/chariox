import assert from "node:assert/strict"
import test from "node:test"

import { createProviderRecoveryController } from "./provider-recovery-controller.js"

type TestSession = {
  id: string
  projectedRunId?: string
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

test("recover relaunches the provider run and reapplies the recovered session", async () => {
  const events: string[] = []
  const controller = createProviderRecoveryController<TestSession, TestProviderRun>({
    isAttached: () => true,
    getSessionId: () => "session-1",
    getProvider: () => "opencode",
    getAccountProfile: () => "default",
    getModel: () => "gpt-5.2",
    getEffort: () => "medium",
    getTargetAgentId: () => "agent-1",
    launchProviderRun: async (input) => {
      assert.deepEqual(input, {
        sessionId: "session-1",
        provider: "opencode",
        accountProfile: "default",
        model: "gpt-5.2",
        effort: "medium",
        targetAgentId: "agent-1",
      })
      events.push("launch")
      return { id: "run-1" }
    },
    getSessionState: async (sessionId) => {
      assert.equal(sessionId, "session-1")
      events.push("session")
      return { id: sessionId }
    },
    projectSession: (session, providerRun) => {
      events.push("project")
      return { ...session, projectedRunId: providerRun.id }
    },
    applyProviderRun: (providerRun) => {
      assert.equal(providerRun.id, "run-1")
      events.push("apply-run")
    },
    applySession: (session) => {
      assert.deepEqual(session, { id: "session-1", projectedRunId: "run-1" })
      events.push("apply-session")
    },
    resizeSession: async (sessionId) => {
      assert.equal(sessionId, "session-1")
      events.push("resize")
    },
    onRecovered: (reason) => {
      assert.equal(reason, "silent_poll")
      events.push("recovered")
    },
    onRecoveryFailed: () => {},
  })

  assert.equal(await controller.recover("silent_poll"), true)
  assert.deepEqual(events, [
    "launch",
    "apply-run",
    "session",
    "project",
    "apply-session",
    "resize",
    "recovered",
  ])
  assert.equal(controller.isInFlight(), false)
})

test("recover is idle when detached or already recovering", async () => {
  const deferred = createDeferred<TestProviderRun>()
  let attached = false
  let launches = 0
  const controller = createProviderRecoveryController<TestSession, TestProviderRun>({
    isAttached: () => attached,
    getSessionId: () => "session-1",
    getProvider: () => "opencode",
    getAccountProfile: () => "default",
    getModel: () => "default",
    getEffort: () => "",
    getTargetAgentId: () => null,
    launchProviderRun: async () => {
      launches += 1
      return deferred.promise
    },
    getSessionState: async () => ({ id: "session-1" }),
    projectSession: (session) => session,
    applyProviderRun: () => {},
    applySession: () => {},
    resizeSession: async () => {},
    onRecovered: () => {},
    onRecoveryFailed: () => {},
  })

  assert.equal(await controller.recover("detached"), false)

  attached = true
  const firstRecovery = controller.recover("silent_poll")
  assert.equal(controller.isInFlight(), true)
  assert.equal(await controller.recover("duplicate"), false)
  deferred.resolve({ id: "run-1" })
  assert.equal(await firstRecovery, true)
  assert.equal(launches, 1)
})

test("recover reports failures and clears in-flight state", async () => {
  let failure: unknown
  const controller = createProviderRecoveryController<TestSession, TestProviderRun>({
    isAttached: () => true,
    getSessionId: () => "session-1",
    getProvider: () => "opencode",
    getAccountProfile: () => "default",
    getModel: () => "default",
    getEffort: () => "",
    getTargetAgentId: () => null,
    launchProviderRun: async () => {
      throw new Error("launch failed")
    },
    getSessionState: async () => ({ id: "session-1" }),
    projectSession: (session) => session,
    applyProviderRun: () => {},
    applySession: () => {},
    resizeSession: async () => {},
    onRecovered: () => {},
    onRecoveryFailed: (_reason, error) => {
      failure = error
    },
  })

  assert.equal(await controller.recover("silent_poll"), false)

  assert.match(failure instanceof Error ? failure.message : String(failure), /launch failed/)
  assert.equal(controller.isInFlight(), false)
})
