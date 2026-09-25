import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import { waitingRoomProjectEnvironmentSetupInput } from "./waiting-room-project-setup.js"

function session(overrides: Record<string, unknown> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-1",
    focused_agent_id: "agent-1",
    agents: [{ id: "agent-1" }],
    ...overrides,
  } as RuntimeSession
}

test("Waiting Room asks the kernel to resolve the selected worker and platform", () => {
  assert.deepEqual(waitingRoomProjectEnvironmentSetupInput(session()), {
    operationId: "waiting-room-project-setup:session-1",
    projectId: "project-1",
    sessionId: "session-1",
    agentId: "agent-1",
    targetWorkerId: "",
    targetPlatform: "",
  })
})

test("Waiting Room rejects a session without a focused Project agent", () => {
  assert.throws(
    () => waitingRoomProjectEnvironmentSetupInput(session({ agents: [] })),
    /no focused agent/,
  )
  assert.throws(
    () => waitingRoomProjectEnvironmentSetupInput(session({ project_id: "" })),
    /no Project binding/,
  )
})
