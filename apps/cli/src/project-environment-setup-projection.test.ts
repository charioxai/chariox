import assert from "node:assert/strict"
import test from "node:test"

import type { ProjectEnvironmentSetupStatus } from "@chariox/kernel-client/kernel-types"
import {
  boundedProgressPercent,
  createProjectEnvironmentSetupProjection,
  safeProjectEnvironmentSetupFailureDetails,
} from "./project-environment-setup-projection.js"

test("project environment setup projection preserves the server phase and bounds progress", async () => {
  const requests: unknown[] = []
  const projection = createProjectEnvironmentSetupProjection({
    client: fakeClient(requests, {
      StartProjectEnvironmentSetup: {
        ProjectEnvironmentSetupStarted: {
          status: status({
            phase: "failed",
            progress_percent: 150,
            failure_code: "setup_failed",
            failure_message: "bad\nworker\u0000detail",
            retryable: true,
          }),
        },
      },
    }),
  })

  const started = await projection.start({
    operationId: "setup-1",
    projectId: "project-1",
    sessionId: "session-1",
    agentId: "agent-1",
    targetWorkerId: "worker-1",
    targetPlatform: "linux-x86_64",
  })

  assert.equal(started.phase, "failed")
  assert.equal(started.progress_percent, 100)
  assert.equal(started.failure_message, "bad worker detail")
  assert.equal(projection.status()?.operation_id, "setup-1")
  assert.deepEqual(requests, [{
    StartProjectEnvironmentSetup: {
      operationId: "setup-1",
      projectId: "project-1",
      sessionId: "session-1",
      agentId: "agent-1",
      targetWorkerId: "worker-1",
      targetPlatform: "linux-x86_64",
    },
  }])
})

test("project environment setup polling does nothing without a persisted operation", async () => {
  const requests: unknown[] = []
  const projection = createProjectEnvironmentSetupProjection({
    client: fakeClient(requests, {}),
  })

  assert.equal(await projection.poll(), null)
  assert.deepEqual(requests, [])
})

test("project environment setup polling recovers the same operation after transport loss", async () => {
  const requests: unknown[] = []
  let connected = false
  const projection = createProjectEnvironmentSetupProjection({
    client: {
      async send<TResponse>(request: unknown) {
        requests.push(request)
        if (!connected) {
          throw new Error("kernel disconnected")
        }
        return {
          ProjectEnvironmentSetupStatus: {
            status: status({ phase: "ready", progress_percent: 100 }),
          },
        } as TResponse
      },
    },
  })
  projection.adopt(status({ phase: "preparing", progress_percent: 20 }))

  assert.equal((await projection.poll())?.phase, "preparing")
  assert.equal(projection.status()?.operation_id, "setup-1")
  assert.match(projection.lastError() ?? "", /kernel disconnected/)

  connected = true
  const recovered = await projection.poll()
  assert.equal(recovered?.operation_id, "setup-1")
  assert.equal(recovered?.phase, "ready")
  assert.equal(projection.lastError(), null)
  assert.deepEqual(requests, [
    { GetProjectEnvironmentSetupStatus: { operationId: "setup-1" } },
    { GetProjectEnvironmentSetupStatus: { operationId: "setup-1" } },
  ])
})

test("project environment setup cancel and retry use the persisted server identity", async () => {
  const requests: unknown[] = []
  const projection = createProjectEnvironmentSetupProjection({
    client: fakeClient(requests, {
      CancelProjectEnvironmentSetup: {
        ProjectEnvironmentSetupCancelled: {
          status: status({
            phase: "cancelled",
            retryable: true,
            session_id: "authoritative-session",
          }),
        },
      },
      RetryProjectEnvironmentSetup: {
        ProjectEnvironmentSetupRetried: {
          status: status({
            phase: "requested",
            attempt: 2,
            progress_percent: 0,
            session_id: "authoritative-session",
          }),
        },
      },
    }),
  })
  projection.adopt(status({ phase: "failed", retryable: true, session_id: "authoritative-session" }))

  assert.equal((await projection.cancel())?.phase, "cancelled")
  assert.equal((await projection.retry())?.attempt, 2)
  assert.deepEqual(requests, [
    {
      CancelProjectEnvironmentSetup: {
        operationId: "setup-1",
        sessionId: "authoritative-session",
      },
    },
    {
      RetryProjectEnvironmentSetup: {
        operationId: "setup-1",
        sessionId: "authoritative-session",
      },
    },
  ])
})

test("project environment setup failure details are bounded and safe for rows", () => {
  assert.equal(boundedProgressPercent(-5), 0)
  assert.equal(boundedProgressPercent(101), 100)
  assert.equal(safeProjectEnvironmentSetupFailureDetails({
    phase: "ready",
    failure_code: "ignored",
    failure_message: "ignored",
  }), null)
  const failure = safeProjectEnvironmentSetupFailureDetails({
    phase: "failed",
    failure_code: "worker_failed",
    failure_message: "line one\nline two\u0000" + "x".repeat(300),
  })
  assert.ok(failure?.startsWith("worker_failed: line one line two x"))
  assert.equal(failure?.length, 240)
})

function fakeClient(requests: unknown[], responses: Record<string, unknown>) {
  return {
    async send<TResponse>(request: unknown) {
      requests.push(request)
      const key = Object.keys(request as Record<string, unknown>)[0] ?? ""
      return responses[key] as TResponse
    },
  }
}

function status(overrides: Partial<ProjectEnvironmentSetupStatus> = {}): ProjectEnvironmentSetupStatus {
  return {
    operation_id: "setup-1",
    project_id: "project-1",
    session_id: "session-1",
    agent_id: "agent-1",
    worker_id: "worker-1",
    platform: "linux-x86_64",
    phase: "requested",
    attempt: 1,
    progress_percent: 0,
    definition_digest: null,
    validation: null,
    message: null,
    failure_code: null,
    failure_message: null,
    retryable: false,
    created_at_ms: 1,
    updated_at_ms: 2,
    ...overrides,
  }
}
