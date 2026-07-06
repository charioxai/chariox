import assert from "node:assert/strict"
import test from "node:test"

import { createProviderRecoveryController } from "./provider-recovery-controller.js"
import { makeAgent, makeSession } from "./command-actions-test-support.js"

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
    getSessionStateSnapshot: () => makeSession({
      agents: [makeAgent({
        id: "agent-1",
        provider: "claude",
        model: "claude/sonnet-4.6",
        effort: "high",
      })],
    }),
    getFallbackLaunch: () => ({
      provider: "opencode",
      model: "gpt-5.2",
      effort: "medium",
    }),
    getAccountProfile: () => "default",
    getTargetAgentId: () => "agent-1",
    launchProviderRun: async (input) => {
      assert.deepEqual(input, {
        sessionId: "session-1",
        provider: "claude",
        accountProfile: "default",
        model: "claude/sonnet-4.6",
        effort: "high",
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
    onRecoverySkipped: () => {},
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
    getSessionStateSnapshot: () => makeSession(),
    getFallbackLaunch: () => ({
      provider: "opencode",
      model: "default",
      effort: "",
    }),
    getAccountProfile: () => "default",
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
    onRecoverySkipped: () => {},
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
    getSessionStateSnapshot: () => makeSession(),
    getFallbackLaunch: () => ({
      provider: "opencode",
      model: "default",
      effort: "",
    }),
    getAccountProfile: () => "default",
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
    onRecoverySkipped: () => {},
    onRecoveryFailed: (_reason, error) => {
      failure = error
    },
  })

  assert.equal(await controller.recover("silent_poll"), false)

  assert.match(failure instanceof Error ? failure.message : String(failure), /launch failed/)
  assert.equal(controller.isInFlight(), false)
})

test("recover skips local launch for remote-backed focused agents", async () => {
  let launches = 0
  let skipped: string | null = null
  const controller = createProviderRecoveryController<TestSession, TestProviderRun>({
    isAttached: () => true,
    getSessionId: () => "session-1",
    getSessionStateSnapshot: () => makeSession({
      agents: [makeAgent({
        id: "agent-1",
        remote_execution: {
          worker_kernel_id: "worker-1",
          worker_machine_id: "machine-1",
          execution_lease_id: "lease-1",
          leased_agent_id: "worker-agent-1",
        },
      })],
    }),
    getFallbackLaunch: () => ({
      provider: "opencode",
      model: "default",
      effort: "",
    }),
    getAccountProfile: () => "default",
    getTargetAgentId: () => "agent-1",
    launchProviderRun: async () => {
      launches += 1
      return { id: "run-1" }
    },
    getSessionState: async () => ({ id: "session-1" }),
    projectSession: (session) => session,
    applyProviderRun: () => {},
    applySession: () => {},
    resizeSession: async () => {},
    onRecovered: () => {},
    onRecoverySkipped: (_reason, skipReason) => {
      skipped = skipReason
    },
    onRecoveryFailed: () => {},
  })

  assert.equal(await controller.recover("silent_poll"), false)

  assert.equal(launches, 0)
  assert.equal(skipped, "remote_backed_agent")
  assert.equal(controller.isInFlight(), false)
})
