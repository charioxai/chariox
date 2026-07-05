import { resolveVisibleTranscriptAgentId } from "./session-runtime-transition.js"

export type ResponsePaneAgent = {
  id: string
}

export type ResponsePaneSelection<T extends ResponsePaneAgent> = {
  visibleAgents: T[]
  primary: T | null
  secondary: T | null
  tertiary: T | null
  visibleTranscriptAgentId: string | null
  screenIndex: number
  screenCount: number
}

export function selectResponsePaneAgents<T extends ResponsePaneAgent>(
  agents: readonly T[],
  focusedAgentId: string | null | undefined,
  split: boolean,
  maxAgentsPerScreen = 3,
): ResponsePaneSelection<T> {
  const firstAgent = agents[0] ?? null

  if (split) {
    const perScreen = Math.max(1, Math.floor(maxAgentsPerScreen))
    const focusedIndex = Math.max(
      0,
      agents.findIndex((agent) => agent.id === focusedAgentId),
    )
    const screenIndex = Math.floor(focusedIndex / perScreen)
    const screenCount = Math.max(1, Math.ceil(agents.length / perScreen))
    const visibleAgents = agents.slice(
      screenIndex * perScreen,
      screenIndex * perScreen + perScreen,
    )
    const primary = visibleAgents[0] ?? null
    const secondary = visibleAgents[1] ?? null
    const tertiary = visibleAgents[2] ?? null
    return {
      visibleAgents: [...visibleAgents],
      primary,
      secondary,
      tertiary,
      visibleTranscriptAgentId: resolveVisibleTranscriptAgentId(true, primary?.id ?? null, focusedAgentId ?? null),
      screenIndex,
      screenCount,
    }
  }

  const focused = agents.find((agent) => agent.id === focusedAgentId) ?? firstAgent
  return {
    visibleAgents: focused ? [focused] : [],
    primary: focused,
    secondary: null,
    tertiary: null,
    visibleTranscriptAgentId: focused?.id ?? null,
    screenIndex: 0,
    screenCount: focused ? 1 : 0,
  }
}

export function splitPaneAuxiliaryAgentIds<T extends ResponsePaneAgent>(
  agents: readonly T[],
  focusedAgentId: string | null | undefined,
  split: boolean,
  maxAgentsPerScreen = 3,
) {
  if (!split) {
    return []
  }
  const selection = selectResponsePaneAgents(agents, focusedAgentId, true, maxAgentsPerScreen)
  return selection.visibleAgents
    .slice(1)
    .map((agent) => agent.id)
}

export function responsePaneBindingsMatch<T extends ResponsePaneAgent>(
  left: Pick<ResponsePaneSelection<T>, "visibleAgents" | "visibleTranscriptAgentId">,
  right: Pick<ResponsePaneSelection<T>, "visibleAgents" | "visibleTranscriptAgentId">,
) {
  return left.visibleTranscriptAgentId === right.visibleTranscriptAgentId
    && left.visibleAgents.length === right.visibleAgents.length
    && left.visibleAgents.every((agent, index) => agent.id === right.visibleAgents[index]?.id)
}
