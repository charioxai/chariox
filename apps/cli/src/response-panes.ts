import { resolveVisibleTranscriptAgentId } from "./runtime.js"

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

export type SplitPaneGeometry = {
  showSecondaryPane: boolean
  showTertiaryPane: boolean
  splitPaneWidth: number
  layoutDirection: "row" | "column"
  layoutGap: number
  topRowVisible: boolean
  topRowGap: number
  topRowFlexBasis: 0 | "auto"
  topRowMinHeight: 0 | null
  primaryFlexGrow: 0 | 1
  primaryWidth: number | "auto"
  primaryFlexBasis: number | "auto"
  primaryMinWidth: number | null
  primaryMaxWidth: number | null
  secondaryWidth: number
  secondaryFlexBasis: number
  secondaryMinWidth: number
  secondaryMaxWidth: number
  tertiaryWidth: 0 | "auto"
  tertiaryFlexGrow: 0 | 1
  tertiaryFlexBasis: 0
  tertiaryMinHeight: 0
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

export function responsePaneRowSlots(maxAgentsPerScreen: number) {
  const slotCount = Math.max(1, Math.floor(maxAgentsPerScreen))
  const rows: number[][] = []
  for (let index = 0; index < slotCount; index += 2) {
    const row = [index]
    if (index + 1 < slotCount) {
      row.push(index + 1)
    }
    rows.push(row)
  }
  return rows
}

export function computeSplitPaneGeometry(
  width: number,
  split: boolean,
  secondaryAgentPresent: boolean,
  tertiaryAgentPresent: boolean,
): SplitPaneGeometry {
  const showSecondaryPane = split && secondaryAgentPresent
  const showTertiaryPane = split && tertiaryAgentPresent
  const fullPaneWidth = Math.max(40, width - 8)
  const splitPaneWidth = Math.max(24, Math.floor(Math.max(40, width - 8) / 2))

  return {
    showSecondaryPane,
    showTertiaryPane,
    splitPaneWidth,
    layoutDirection: showTertiaryPane ? "column" : "row",
    layoutGap: split && (showSecondaryPane || showTertiaryPane) ? 1 : 0,
    topRowVisible: true,
    topRowGap: showSecondaryPane ? 1 : 0,
    topRowFlexBasis: showTertiaryPane ? 0 : "auto",
    topRowMinHeight: showTertiaryPane ? 0 : null,
    primaryFlexGrow: split && showSecondaryPane ? 0 : 1,
    primaryWidth: split && showSecondaryPane ? splitPaneWidth : fullPaneWidth,
    primaryFlexBasis: split && showSecondaryPane ? splitPaneWidth : fullPaneWidth,
    primaryMinWidth: split && showSecondaryPane ? splitPaneWidth : fullPaneWidth,
    primaryMaxWidth: split && showSecondaryPane ? splitPaneWidth : fullPaneWidth,
    secondaryWidth: showSecondaryPane ? splitPaneWidth : 0,
    secondaryFlexBasis: showSecondaryPane ? splitPaneWidth : 0,
    secondaryMinWidth: showSecondaryPane ? splitPaneWidth : 0,
    secondaryMaxWidth: showSecondaryPane ? splitPaneWidth : 0,
    tertiaryWidth: showTertiaryPane ? "auto" : 0,
    tertiaryFlexGrow: showTertiaryPane ? 1 : 0,
    tertiaryFlexBasis: 0,
    tertiaryMinHeight: 0,
  }
}
