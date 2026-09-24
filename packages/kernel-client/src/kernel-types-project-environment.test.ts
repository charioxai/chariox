import assert from "node:assert/strict"
import test from "node:test"

import type {
  ProjectEnvironmentSetupPhase,
  ProjectEnvironmentSetupStatus,
} from "./kernel-types-project-environment.js"

test("project environment setup status keeps the kernel-owned lifecycle phases and progress field", () => {
  const phases: ProjectEnvironmentSetupPhase[] = [
    "requested",
    "preparing",
    "validating",
    "ready",
    "failed",
    "cancelled",
  ]
  const status: ProjectEnvironmentSetupStatus = {
    operation_id: "setup-1",
    project_id: "project-1",
    session_id: "session-1",
    agent_id: "agent-1",
    worker_id: "worker-1",
    platform: "linux-x86_64",
    phase: "validating",
    attempt: 1,
    progress_percent: 50,
    definition_digest: null,
    validation: null,
    message: "validating",
    failure_code: null,
    failure_message: null,
    retryable: false,
    created_at_ms: 1,
    updated_at_ms: 2,
  }

  assert.deepEqual(phases, [
    "requested",
    "preparing",
    "validating",
    "ready",
    "failed",
    "cancelled",
  ])
  assert.equal(status.progress_percent, 50)
  assert.equal(status.phase, "validating")
})
