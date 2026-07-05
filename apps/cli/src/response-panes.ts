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

export function workflowCanvasPaneIndices(options: {
  split: boolean
  visibleAgentCount: number
  screenIndex: number
  screenCount: number
  maxAgentsPerScreen: number
}) {
  if (!options.split || options.screenCount < 1 || options.screenIndex !== options.screenCount - 1) {
    return []
  }

  const slotCount = Math.max(1, Math.floor(options.maxAgentsPerScreen))
  const workflowSlotCount = Math.max(0, slotCount - options.visibleAgentCount)
  return Array.from({ length: workflowSlotCount }, (_, index) => options.visibleAgentCount + index)
}

export function computeSplitPaneGeometry(
  width: number,
  split: boolean,
  secondaryAgentPresent: boolean,
  tertiaryAgentPresent: boolean,
): SplitPaneGeometry {
  const showSecondaryPane = split && secondaryAgentPresent
  const showTertiaryPane = split && tertiaryAgentPresent
  const fullPaneWidth = Math.max(40, width)
  const splitPaneWidth = Math.max(24, Math.floor(fullPaneWidth / 2))

  return {
    showSecondaryPane,
    showTertiaryPane,
    splitPaneWidth,
    layoutDirection: showTertiaryPane ? "column" : "row",
    layoutGap: 0,
    topRowVisible: true,
    topRowGap: 0,
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
