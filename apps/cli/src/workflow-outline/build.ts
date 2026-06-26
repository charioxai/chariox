import type { AgentInstance, WorkflowDefinition, WorkflowNodeDefinition, WorkflowRun } from "../cli-types.js"
import type { WorkflowComponentSelection } from "../workflow-component-selection.js"
import { collaboratorAgentLabel, workflowAgentRefDisplayLabel } from "../workflow-collaboration-labels.js"
import type { WorkflowOutline, WorkflowOutlineEdgeItem, WorkflowOutlineNodeItem } from "./types.js"

export function buildWorkflowOutline(options: {
  workflow: WorkflowDefinition
  agents: AgentInstance[]
  workflowRuns?: WorkflowRun[]
  selectedNodeId: string | null
  selectedComponent?: WorkflowComponentSelection | null
}): WorkflowOutline {
  const nodes = options.workflow.nodes ?? []
  const edges = options.workflow.edges ?? []
  const endpoints = options.workflow.endpoints ?? []
  const agentById = new Map(options.agents.map((agent) => [agent.id, agent] as const))
  const nodeById = new Map(nodes.map((node) => [node.id, node] as const))
  const nodeOrder = new Map(nodes.map((node, index) => [node.id, index] as const))
  const displayRun = resolveWorkflowDisplayRun(options.workflow.id, options.workflowRuns ?? [])
  const nodeRunStatusByNodeId = new Map(
    (displayRun?.node_runs ?? []).map((nodeRun) => [nodeRun.node_id, nodeRun.status] as const),
  )
  const nodeIdByNodeRunId = new Map(
    (displayRun?.node_runs ?? []).map((nodeRun) => [nodeRun.id, nodeRun.node_id] as const),
  )
  const selectedNodeFailures = (displayRun?.failure_events ?? [])
    .filter((event) => nodeIdByNodeRunId.get(event.source_node_run_id) === options.selectedNodeId)
    .sort((left, right) => right.timestamp_ms - left.timestamp_ms)
  const failureCountByNodeId = new Map<string, number>()
  for (const event of displayRun?.failure_events ?? []) {
    const nodeId = nodeIdByNodeRunId.get(event.source_node_run_id)
    if (!nodeId) {
      continue
    }
    failureCountByNodeId.set(nodeId, (failureCountByNodeId.get(nodeId) ?? 0) + 1)
  }
  const endpointIndexById = new Map(endpoints.map((endpoint, index) => [endpoint.id, index] as const))
  const edgeIndexById = new Map(edges.map((edge, index) => [edge.id, index] as const))
  const nodesOutline: WorkflowOutlineNodeItem[] = nodes.map((node) => {
    const agent = agentById.get(node.agent_id) ?? null
    const recentFailures = node.id === options.selectedNodeId
      ? selectedNodeFailures.slice(0, 3).map((event) => ({
        kind: event.kind,
        message: event.message,
        timestampMs: event.timestamp_ms,
      }))
      : []
    return {
      id: node.id,
      agentId: node.agent_id,
      agentRef: workflowAgentRefDisplayLabel(agent),
      agentAlias: agent?.alias ?? null,
      provider: agent?.provider ?? null,
      model: agent?.model ?? null,
      effort: agent?.effort ?? null,
      runStatus: nodeRunStatusByNodeId.get(node.id) ?? null,
      instructions: node.instructions ?? null,
      missing: !agent,
      selected: node.id === options.selectedNodeId,
      selectedComponent: options.selectedComponent?.kind === "node" && options.selectedComponent.id === node.id,
      outgoingEdges: edges
        .filter((edge) => edge.from_node_id === node.id)
        .sort((left, right) => compareEdgeTargets(left.to_node_id, right.to_node_id, nodeOrder) || compareByMapIndex(left.id, right.id, edgeIndexById))
        .map((edge) => buildEdgeItem(edge.id, edge.from_node_id, edge.to_node_id, nodeById, agentById)),
      incomingEdges: edges
        .filter((edge) => edge.to_node_id === node.id)
        .sort((left, right) => compareEdgeTargets(left.from_node_id, right.from_node_id, nodeOrder) || compareByMapIndex(left.id, right.id, edgeIndexById))
        .map((edge) => buildEdgeItem(edge.id, edge.from_node_id, edge.from_node_id, nodeById, agentById)),
      entryEndpoints: endpoints
        .filter((endpoint) => endpoint.entry_node_id === node.id)
        .sort((left, right) => compareByMapIndex(left.id, right.id, endpointIndexById))
        .map((endpoint) => ({
          id: endpoint.id,
          alias: endpoint.alias,
          entryNodeId: endpoint.entry_node_id,
        })),
      failureCount: failureCountByNodeId.get(node.id) ?? 0,
      recentFailures,
    }
  })

  return {
    workflowId: options.workflow.id,
    workflowAlias: options.workflow.alias,
    workflowRunId: displayRun?.id ?? null,
    workflowRunStatus: displayRun?.status ?? null,
    workflowRunFinalOutput: formatWorkflowRunFinalOutput(displayRun ?? null),
    workflowRunFinalOutputValid: displayRun?.final_output_valid ?? null,
    workflowFailureCount: displayRun?.failure_events?.length ?? 0,
    edgeCount: edges.length,
    endpointCount: endpoints.length,
    nodeCount: nodes.length,
    agentLabels: options.agents.map((agent) => agent.agent_ref ?? agent.id),
    nodes: nodesOutline,
  }
}

function buildEdgeItem(
  edgeId: string,
  fromNodeId: string,
  adjacentNodeId: string,
  nodeById: Map<string, WorkflowNodeDefinition>,
  agentById: Map<string, AgentInstance>,
): WorkflowOutlineEdgeItem {
  const adjacentNode = nodeById.get(adjacentNodeId)
  const adjacentAgent = adjacentNode ? agentById.get(adjacentNode.agent_id) ?? null : null
  return {
    id: edgeId,
    fromNodeId,
    nodeId: adjacentNodeId,
    agentId: adjacentNode?.agent_id ?? adjacentNodeId,
    agentRef: adjacentNode ? workflowAgentRefDisplayLabel(adjacentAgent) : collaboratorAgentLabel,
    agentAlias: adjacentAgent?.alias ?? null,
  }
}

function compareEdgeTargets(
  leftNodeId: string,
  rightNodeId: string,
  nodeOrder: Map<string, number>,
) {
  return (nodeOrder.get(leftNodeId) ?? Number.MAX_SAFE_INTEGER) - (nodeOrder.get(rightNodeId) ?? Number.MAX_SAFE_INTEGER)
}

function compareByMapIndex(leftId: string, rightId: string, indexById: Map<string, number>) {
  return (indexById.get(leftId) ?? Number.MAX_SAFE_INTEGER) - (indexById.get(rightId) ?? Number.MAX_SAFE_INTEGER)
}

function resolveWorkflowDisplayRun(workflowId: string, workflowRuns: WorkflowRun[]) {
  const matchingRuns = workflowRuns.filter((workflowRun) => workflowRun.workflow_id === workflowId)
  if (matchingRuns.length === 0) {
    return null
  }
  const nonTerminalRuns = matchingRuns.filter((workflowRun) => !isTerminalWorkflowRunStatus(workflowRun.status))
  const candidates = nonTerminalRuns.length > 0 ? nonTerminalRuns : matchingRuns
  return [...candidates].sort((left, right) => right.created_at_ms - left.created_at_ms)[0] ?? null
}

function isTerminalWorkflowRunStatus(status: string) {
  return status === "Completed" || status === "Failed" || status === "Stopped"
}

function formatWorkflowRunFinalOutput(workflowRun: WorkflowRun | null) {
  const message = workflowRun?.final_output?.message
  if (!message) {
    return null
  }
  const singleLine = message.replace(/\s+/g, " ").trim()
  if (singleLine.length <= 180) {
    return singleLine
  }
  return `${singleLine.slice(0, 177)}...`
}
