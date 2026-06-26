export type WorkflowOutlineEdgeItem = {
  id: string
  fromNodeId: string
  nodeId: string
  agentId: string
  agentRef: string
  agentAlias: string | null
}

export type WorkflowOutlineEndpointItem = {
  id: string
  alias: string | null
  entryNodeId: string
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
  selectedComponent: boolean
  outgoingEdges: WorkflowOutlineEdgeItem[]
  incomingEdges: WorkflowOutlineEdgeItem[]
  entryEndpoints: WorkflowOutlineEndpointItem[]
  failureCount: number
  recentFailures: Array<{
    kind: string
    message: string
    timestampMs: number
  }>
}

export type WorkflowOutline = {
  workflowId: string
  workflowAlias: string | null
  workflowRunId: string | null
  workflowRunStatus: string | null
  workflowRunFinalOutput: string | null
  workflowRunFinalOutputValid: boolean | null
  workflowFailureCount: number
  edgeCount: number
  endpointCount: number
  nodeCount: number
  agentLabels: string[]
  nodes: WorkflowOutlineNodeItem[]
}
