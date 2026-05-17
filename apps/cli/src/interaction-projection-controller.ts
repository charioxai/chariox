import type {
  RuntimeInteraction,
  RuntimeSession,
} from "./cli-types.js"
import {
  activeInteractionForAgent as activeInteractionForAgentForSession,
} from "./session-state.js"

type InteractionProjectionControllerDeps = {
  getSession: () => RuntimeSession
  getFocusedAgentId: () => string | null
}

export function createInteractionProjectionController(
  deps: InteractionProjectionControllerDeps,
): {
  activeInteractionForAgent: (agentId: string | null | undefined) => RuntimeInteraction | null
  focusedAgentInteraction: () => RuntimeInteraction | null
} {
  const activeInteractionForAgent = (
    agentId: string | null | undefined,
  ): RuntimeInteraction | null => {
    return activeInteractionForAgentForSession(deps.getSession(), agentId)
  }

  const focusedAgentInteraction = (): RuntimeInteraction | null => {
    return activeInteractionForAgent(deps.getFocusedAgentId())
  }

  return {
    activeInteractionForAgent,
    focusedAgentInteraction,
  }
}
