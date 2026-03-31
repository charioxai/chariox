export type AgentPaneRow = {
  key: string
  slots: Array<number | null>
}

export function buildAgentPaneRows(agentCount: number): AgentPaneRow[] {
  if (agentCount < 1) {
    return []
  }
  if (agentCount === 1) {
    return [{ key: "row-1", slots: [0] }]
  }
  if (agentCount === 2) {
    return [{ key: "row-1", slots: [0, 1] }]
  }
  if (agentCount === 3) {
    return [
      { key: "row-1", slots: [0, 1] },
      { key: "row-2", slots: [2] },
    ]
  }
  if (agentCount === 4) {
    return [
      { key: "row-1", slots: [0, 1] },
      { key: "row-2", slots: [2, 3] },
    ]
  }
  if (agentCount <= 6) {
    return [
      { key: "row-1", slots: [0, 1, 2] },
      { key: "row-2", slots: [3, 4, agentCount === 5 ? null : 5] },
    ]
  }
  throw new Error(`Unsupported agent count: ${agentCount}. Max is 6.`)
}
