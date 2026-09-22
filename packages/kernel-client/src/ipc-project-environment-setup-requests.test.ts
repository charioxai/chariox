import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import {
  cancelProjectEnvironmentSetupRequest,
  getProjectEnvironmentSetupStatusRequest,
  retryProjectEnvironmentSetupRequest,
  startProjectEnvironmentSetupRequest,
} from "./ipc-project-environment-setup-requests.js"

test("project environment setup requests use the versioned kernel seam", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 341)
  assert.deepEqual(
    startProjectEnvironmentSetupRequest({
      operationId: "setup-1",
      projectId: "project-1",
      sessionId: "session-1",
      agentId: "agent-1",
      targetWorkerId: "machine-1",
      targetPlatform: "linux-x86_64",
      validationCommands: ["timeout 180s cargo check --workspace --locked"],
    }),
    {
      StartProjectEnvironmentSetup: {
        operationId: "setup-1",
        projectId: "project-1",
        sessionId: "session-1",
        agentId: "agent-1",
        targetWorkerId: "machine-1",
        targetPlatform: "linux-x86_64",
        validationCommands: ["timeout 180s cargo check --workspace --locked"],
      },
    },
  )
  assert.deepEqual(getProjectEnvironmentSetupStatusRequest("setup-1"), {
    GetProjectEnvironmentSetupStatus: { operationId: "setup-1" },
  })
  assert.deepEqual(cancelProjectEnvironmentSetupRequest("setup-1", "session-1"), {
    CancelProjectEnvironmentSetup: { operationId: "setup-1", sessionId: "session-1" },
  })
  assert.deepEqual(retryProjectEnvironmentSetupRequest("setup-1", "session-1"), {
    RetryProjectEnvironmentSetup: { operationId: "setup-1", sessionId: "session-1" },
  })
})
