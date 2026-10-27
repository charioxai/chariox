import type { ProjectEnvironmentSetupStartInput } from "@chariox/kernel-client/project-environment-setup-orchestration"
import type { RuntimeSession } from "./cli-types.js"

const MAX_OPERATION_ID_CHARS = 128

export function waitingRoomProjectEnvironmentSetupInput(
  session: RuntimeSession,
): ProjectEnvironmentSetupStartInput {
  const sessionId = session.id.trim()
  const projectId = session.project_id.trim()
  const agentId = session.focused_agent_id?.trim() ?? ""
  const operationId = `waiting-room-project-setup:${sessionId}`

  if (!sessionId || operationId.length > MAX_OPERATION_ID_CHARS) {
    throw new Error("Waiting Room Project setup operation identity is invalid")
  }
  if (!projectId) {
    throw new Error("The created session has no Project binding for environment setup")
  }
  if (!agentId || !session.agents.some((agent) => agent.id === agentId)) {
    throw new Error("The created session has no focused agent for Project environment setup")
  }

  return {
    operationId,
    projectId,
    sessionId,
    agentId,
    targetWorkerId: "",
    targetPlatform: "",
  }
}
