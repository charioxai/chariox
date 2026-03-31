import { resolveVisibleTranscriptAgentId } from "./runtime.js"

export type ResponsePaneAgent = {
  id: string
}

export type ResponsePaneSelection<T extends ResponsePaneAgent> = {
  primary: T | null
  secondary: T | null
  tertiary: T | null
  visibleTranscriptAgentId: string | null
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
): ResponsePaneSelection<T> {
  const primary = agents[0] ?? null
  const secondary = split ? (agents[1] ?? null) : null
  const tertiary = split ? (agents[2] ?? null) : null

  if (split) {
    return {
      primary,
      secondary,
      tertiary,
      visibleTranscriptAgentId: resolveVisibleTranscriptAgentId(true, primary?.id ?? null, focusedAgentId ?? null),
    }
  }

  const focused = agents.find((agent) => agent.id === focusedAgentId) ?? primary
  return {
    primary: focused,
    secondary: null,
    tertiary: null,
    visibleTranscriptAgentId: focused?.id ?? null,
  }
}

export function splitPaneAuxiliaryAgentIds<T extends ResponsePaneAgent>(
  agents: readonly T[],
  split: boolean,
) {
  if (!split) {
    return []
  }
  return agents.slice(1, 3).map((agent) => agent.id)
}

export function computeSplitPaneGeometry(
  width: number,
  split: boolean,
  secondaryAgentPresent: boolean,
  tertiaryAgentPresent: boolean,
): SplitPaneGeometry {
  const showSecondaryPane = split && secondaryAgentPresent
  const showTertiaryPane = split && tertiaryAgentPresent
  const splitPaneWidth = Math.max(24, Math.floor(Math.max(40, width - 8) / 2))

  return {
    showSecondaryPane,
    showTertiaryPane,
    splitPaneWidth,
    layoutDirection: showTertiaryPane ? "column" : "row",
    layoutGap: split && (showSecondaryPane || showTertiaryPane) ? 1 : 0,
    topRowVisible: split ? (showSecondaryPane || showTertiaryPane) : true,
    topRowGap: showSecondaryPane ? 1 : 0,
    topRowFlexBasis: showTertiaryPane ? 0 : "auto",
    topRowMinHeight: showTertiaryPane ? 0 : null,
    primaryFlexGrow: split && showSecondaryPane ? 0 : 1,
    primaryWidth: split && showSecondaryPane ? splitPaneWidth : "auto",
    primaryFlexBasis: split && showSecondaryPane ? splitPaneWidth : "auto",
    primaryMinWidth: split && showSecondaryPane ? splitPaneWidth : null,
    primaryMaxWidth: split && showSecondaryPane ? splitPaneWidth : null,
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
