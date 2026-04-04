export type WorkflowOutlineEdgeItem = {
  id: string
  nodeId: string
  agentId: string
  agentRef: string
  agentAlias: string | null
}

export type WorkflowOutlineEndpointItem = {
  id: string
  alias: string | null
}

export type WorkflowOutlineNodeItem = {
  id: string
  agentId: string
  agentRef: string
  agentAlias: string | null
  provider: string | null
  model: string | null
  effort: string | null
  runStatus: string | null
  instructions: string | null
  missing: boolean
  selected: boolean
  outgoingEdges: WorkflowOutlineEdgeItem[]
  incomingEdges: WorkflowOutlineEdgeItem[]
  entryEndpoints: WorkflowOutlineEndpointItem[]
}

export type WorkflowOutline = {
  workflowId: string
  workflowAlias: string | null
  workflowRunId: string | null
  workflowRunStatus: string | null
  edgeCount: number
  endpointCount: number
  nodeCount: number
  agentLabels: string[]
  nodes: WorkflowOutlineNodeItem[]
}

