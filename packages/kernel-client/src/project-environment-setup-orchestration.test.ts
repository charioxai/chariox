import assert from "node:assert/strict"
import test from "node:test"

import type { ProjectEnvironmentSetupStatus } from "./kernel-types-project-environment.js"
import {
  ensureProjectEnvironmentSetupReady,
  managedLaunchProjectEnvironmentSetupOperationId,
  type ProjectEnvironmentSetupStartInput,
} from "./project-environment-setup-orchestration.js"

const input: ProjectEnvironmentSetupStartInput = {
  operationId: "managed-launch-project-setup:session-1",
  projectId: "project-1",
  sessionId: "session-1",
  agentId: "agent-1",
  targetWorkerId: "worker-1",
  targetPlatform: "linux-x86_64",
}

test("managed launch setup operation identity is stable for a session", () => {
  assert.equal(
    managedLaunchProjectEnvironmentSetupOperationId("session-1"),
    "managed-launch-project-setup:session-1",
  )
})

test("setup readiness recovers a lost start response without starting a second operation", async () => {
  const calls: string[] = []
  const statuses: ProjectEnvironmentSetupStatus[] = []
  const ready = await ensureProjectEnvironmentSetupReady({
    async start() {
      calls.push("start")
      throw retryableTransportError()
    },
    async get(operationId) {
      calls.push(`get:${operationId}`)
      return status({ phase: "ready", progress_percent: 100 })
    },
  }, input, immediateOptions(statuses))

  assert.equal(ready.phase, "ready")
  assert.deepEqual(calls, ["start", `get:${input.operationId}`])
  assert.deepEqual(statuses.map((candidate) => candidate.phase), ["ready"])
})

test("setup readiness remains pending across a retryable failure and observes same-operation retry", async () => {
  const statuses: ProjectEnvironmentSetupStatus[] = []
  const pending = [
    status({ phase: "requested", attempt: 2 }),
    status({ phase: "ready", attempt: 2, progress_percent: 100 }),
  ]
  const ready = await ensureProjectEnvironmentSetupReady({
    async start() {
      return status({
        phase: "failed",
        failure_code: "setup_failed",
        failure_message: "retry me",
        retryable: true,
      })
    },
    async get() {
      return pending.shift() as ProjectEnvironmentSetupStatus
    },
  }, input, immediateOptions(statuses))

  assert.equal(ready.attempt, 2)
  assert.deepEqual(statuses.map((candidate) => candidate.phase), ["failed", "requested", "ready"])
})

test("setup readiness rejects cancellation and stale launch identity", async () => {
  await assert.rejects(
    ensureProjectEnvironmentSetupReady({
      async start() {
        return status({ phase: "cancelled", retryable: true })
      },
      async get() {
        throw new Error("unexpected get")
      },
    }, input, immediateOptions([])),
    /cancelled/,
  )

  await assert.rejects(
    ensureProjectEnvironmentSetupReady({
      async start() {
        return status({ project_id: "stale-project", phase: "ready" })
      },
      async get() {
        throw new Error("unexpected get")
      },
    }, input, immediateOptions([])),
    /changed its launch binding/,
  )

  await assert.rejects(
    ensureProjectEnvironmentSetupReady({
      async start() {
        return status({
          phase: "failed",
          failure_code: "not_retryable",
          failure_message: "cannot prepare Project",
        })
      },
      async get() {
        throw new Error("unexpected get")
      },
    }, input, immediateOptions([])),
    /cannot prepare Project/,
  )
})

test("setup readiness pins the worker and platform resolved by the kernel", async () => {
  const selectedByKernel = { ...input, targetWorkerId: "", targetPlatform: "" }
  const pending = [
    status({ phase: "preparing", worker_id: "worker-2", platform: "linux-arm64" }),
    status({ phase: "ready", worker_id: "worker-2", platform: "linux-arm64" }),
  ]
  const ready = await ensureProjectEnvironmentSetupReady({
    async start() {
      return status({ phase: "requested", worker_id: "worker-2", platform: "linux-arm64" })
    },
    async get() {
      return pending.shift() as ProjectEnvironmentSetupStatus
    },
  }, selectedByKernel, immediateOptions([]))

  assert.equal(ready.worker_id, "worker-2")
  assert.equal(ready.platform, "linux-arm64")
})

test("setup readiness rejects a changed or empty kernel-resolved target", async () => {
  const selectedByKernel = { ...input, targetWorkerId: "", targetPlatform: "" }
  await assert.rejects(
    ensureProjectEnvironmentSetupReady({
      async start() {
        return status({ worker_id: "", platform: "" })
      },
      async get() {
        throw new Error("unexpected get")
      },
    }, selectedByKernel, immediateOptions([])),
    /did not resolve the selected worker and platform/,
  )

  await assert.rejects(
    ensureProjectEnvironmentSetupReady({
      async start() {
        return status({ worker_id: "worker-2", platform: "linux-arm64" })
      },
      async get() {
        return status({ phase: "ready", worker_id: "worker-3", platform: "linux-arm64" })
      },
    }, selectedByKernel, immediateOptions([])),
    /changed its launch binding/,
  )
})

function immediateOptions(statuses: ProjectEnvironmentSetupStatus[]) {
  return {
    delay: async () => {},
    isRetryableTransportError: (error: unknown) => (
      error instanceof Error && "retryable" in error && error.retryable === true
    ),
    onStatus: (candidate: ProjectEnvironmentSetupStatus) => statuses.push(candidate),
  }
}

function retryableTransportError() {
  return Object.assign(new Error("connection lost"), { retryable: true })
}

function status(overrides: Partial<ProjectEnvironmentSetupStatus> = {}): ProjectEnvironmentSetupStatus {
  return {
    operation_id: input.operationId,
    project_id: input.projectId,
    session_id: input.sessionId,
    agent_id: input.agentId,
    worker_id: input.targetWorkerId,
    platform: input.targetPlatform,
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
