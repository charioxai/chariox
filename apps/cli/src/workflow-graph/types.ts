export type WorkflowGraphMetrics = {
  nodeWidth: number
  nodeHeight: number
  horizontalGap: number
  verticalGap: number
  componentGap: number
  endpointGap: number
}

export type WorkflowGraphNodeLayout = {
  id: string
  agentId: string
  alias: string | null
  provider: string | null
  model: string | null
  effort: string | null
  missing: boolean
  selected: boolean
  x: number
  y: number
  width: number
  height: number
  lines: string[]
}

export type WorkflowGraphEdgeLayout = {
  id: string
  fromNodeId: string
  toNodeId: string
  points: Array<{ x: number; y: number }>
}

export type WorkflowGraphEndpointLayout = {
  id: string
  alias: string | null
  entryNodeId: string
  markerX: number
  markerY: number
  labelX: number
  labelY: number
  label: string
}

export type WorkflowGraphLayout = {
  workflowId: string
  workflowAlias: string | null
  width: number
  height: number
  nodes: WorkflowGraphNodeLayout[]
  edges: WorkflowGraphEdgeLayout[]
  endpoints: WorkflowGraphEndpointLayout[]
}
