import type { RuntimeSession } from "./kernel-types.js"

export function sessionWithProjectedAgentActivity(payload: {
  session: RuntimeSession
  agent_activity?: RuntimeSession["agent_activity"] | null
  agent_activity_revision?: number | null
}): RuntimeSession {
  if (!payload.agent_activity) {
    return payload.session
  }
  return {
    ...payload.session,
    agent_activity: payload.agent_activity,
    ...(typeof payload.agent_activity_revision === "number"
      ? { agent_activity_revision: payload.agent_activity_revision }
      : {}),
  }
}
