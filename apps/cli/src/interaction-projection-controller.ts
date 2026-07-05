import type {
  RuntimeInteraction,
  RuntimeSession,
} from "./cli-types.js"
import {
  sessionActiveInteractionForAgent,
} from "@arroba/kernel-client/session-runtime-lookup"

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
    return sessionActiveInteractionForAgent(deps.getSession(), agentId) as RuntimeInteraction | null
  }

  const focusedAgentInteraction = (): RuntimeInteraction | null => {
    return activeInteractionForAgent(deps.getFocusedAgentId())
  }

  return {
    activeInteractionForAgent,
    focusedAgentInteraction,
  }
}
