import type { AgentInstance, WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowNodeDefinition } from "../cli-types.js"
import { routeWorkflowEdge } from "./routing.js"
import type {
  WorkflowGraphEdgeLayout,
  WorkflowGraphEndpointLayout,
  WorkflowGraphLayout,
  WorkflowGraphMetrics,
  WorkflowGraphNodeLayout,
} from "./types.js"

const BASE_NODE_WIDTH = 30
const BASE_NODE_HEIGHT = 8
const BASE_HORIZONTAL_GAP = 8
const BASE_VERTICAL_GAP = 5
const BASE_COMPONENT_GAP = 7
const GRAPH_PADDING_X = 4
const GRAPH_PADDING_Y = 3
const MIN_NODE_WIDTH = 24
const MIN_NODE_HEIGHT = 7

export function resolveWorkflowGraphMetrics(): WorkflowGraphMetrics {
  return {
    nodeWidth: Math.max(MIN_NODE_WIDTH, BASE_NODE_WIDTH),
    nodeHeight: Math.max(MIN_NODE_HEIGHT, BASE_NODE_HEIGHT),
    horizontalGap: Math.max(4, BASE_HORIZONTAL_GAP),
    verticalGap: Math.max(3, BASE_VERTICAL_GAP),
    componentGap: Math.max(4, BASE_COMPONENT_GAP),
    endpointGap: Math.max(2, 3),
  }
}

export function buildWorkflowGraphLayout(options: {
  workflow: WorkflowDefinition
  agents: AgentInstance[]
  selectedNodeId: string | null
}): WorkflowGraphLayout {
  const metrics = resolveWorkflowGraphMetrics()
  const nodes = options.workflow.nodes ?? []
  const edges = options.workflow.edges ?? []
  const endpoints = options.workflow.endpoints ?? []
  const agentById = new Map(options.agents.map((agent) => [agent.id, agent] as const))
  const nodeById = new Map(nodes.map((node) => [node.id, node] as const))
  const componentIds = computeWeaklyConnectedComponents(nodes, edges)
  const layoutNodes: WorkflowGraphNodeLayout[] = []
  const layoutEdges: WorkflowGraphEdgeLayout[] = []
  const layoutEndpoints: WorkflowGraphEndpointLayout[] = []
  let nextComponentTop = GRAPH_PADDING_Y
  let graphWidth = GRAPH_PADDING_X * 2

  for (const componentNodeIds of componentIds) {
    const componentNodes = componentNodeIds
      .map((nodeId) => nodeById.get(nodeId))
      .filter((node): node is WorkflowNodeDefinition => Boolean(node))
    if (componentNodes.length === 0) {
      continue
    }
    const componentEdges = edges.filter(
      (edge) => componentNodeIds.includes(edge.from_node_id) && componentNodeIds.includes(edge.to_node_id),
    )
    const componentEndpoints = endpoints.filter((endpoint) => componentNodeIds.includes(endpoint.entry_node_id))
    const ranks = assignNodeRanks(componentNodes, componentEdges, componentEndpoints)
    const levels = groupNodesByRank(componentNodes, ranks)
    const componentWidth = levels.reduce((max, level) => {
      const levelWidth = level.length * metrics.nodeWidth + Math.max(0, level.length - 1) * metrics.horizontalGap
      return Math.max(max, levelWidth)
    }, metrics.nodeWidth)
    const componentTop = nextComponentTop
    const componentLeft = GRAPH_PADDING_X
    const nodeLayoutById = new Map<string, WorkflowGraphNodeLayout>()

    levels.forEach((levelNodes, levelIndex) => {
      const levelWidth = levelNodes.length * metrics.nodeWidth + Math.max(0, levelNodes.length - 1) * metrics.horizontalGap
      let currentX = componentLeft + Math.max(0, Math.floor((componentWidth - levelWidth) / 2))
      const currentY = componentTop + levelIndex * (metrics.nodeHeight + metrics.verticalGap) + metrics.endpointGap + 1
      for (const node of levelNodes) {
        const agent = agentById.get(node.agent_id) ?? null
        const provider = agent?.provider ?? null
        const model = agent?.model ?? null
        const effort = agent?.effort ?? null
        const layoutNode = {
          id: node.id,
          agentId: node.agent_id,
          alias: agent?.alias ?? null,
          provider,
          model,
          effort,
          missing: !agent,
          selected: node.id === options.selectedNodeId,
          x: currentX,
          y: currentY,
          width: metrics.nodeWidth,
          height: metrics.nodeHeight,
          lines: formatNodeLines({
            title: agent
              ? (agent.alias ? `${agent.agent_ref} (${agent.alias})` : agent.agent_ref)
              : node.agent_id,
            provider,
            model,
            effort,
          }, metrics.nodeWidth),
        } satisfies WorkflowGraphNodeLayout
        nodeLayoutById.set(node.id, layoutNode)
        layoutNodes.push(layoutNode)
        currentX += metrics.nodeWidth + metrics.horizontalGap
      }
    })

    for (const endpoint of componentEndpoints) {
      const node = nodeLayoutById.get(endpoint.entry_node_id)
      if (!node) {
        continue
      }
      const label = endpoint.alias ? `${endpoint.id} (${endpoint.alias})` : endpoint.id
      layoutEndpoints.push({
        id: endpoint.id,
        alias: endpoint.alias,
        entryNodeId: endpoint.entry_node_id,
        markerX: node.x + Math.floor(node.width / 2),
        markerY: node.y - metrics.endpointGap - 1,
        labelX: node.x + Math.max(0, Math.floor((node.width - label.length) / 2)),
        labelY: node.y - metrics.endpointGap - 2,
        label,
      })
    }

    for (const edge of componentEdges) {
      const fromNode = nodeLayoutById.get(edge.from_node_id)
      const toNode = nodeLayoutById.get(edge.to_node_id)
      if (!fromNode || !toNode) {
        continue
      }
      layoutEdges.push({
        id: edge.id,
        fromNodeId: edge.from_node_id,
        toNodeId: edge.to_node_id,
        points: routeWorkflowEdge(fromNode, toNode),
      })
    }

    const componentHeight = levels.length * metrics.nodeHeight
      + Math.max(0, levels.length - 1) * metrics.verticalGap
      + metrics.endpointGap
      + 2
    nextComponentTop += componentHeight + metrics.componentGap
    graphWidth = Math.max(graphWidth, componentLeft + componentWidth + GRAPH_PADDING_X)
  }

  const graphHeight = Math.max(GRAPH_PADDING_Y * 2 + 6, nextComponentTop + GRAPH_PADDING_Y - metrics.componentGap)

  return {
    workflowId: options.workflow.id,
    workflowAlias: options.workflow.alias,
    width: graphWidth,
    height: graphHeight,
    nodes: layoutNodes,
    edges: layoutEdges,
    endpoints: layoutEndpoints,
  }
}

function computeWeaklyConnectedComponents(
  nodes: WorkflowNodeDefinition[],
  edges: WorkflowEdgeDefinition[],
) {
  const adjacency = new Map<string, Set<string>>()
  for (const node of nodes) {
    adjacency.set(node.id, new Set())
  }
  for (const edge of edges) {
    adjacency.get(edge.from_node_id)?.add(edge.to_node_id)
    adjacency.get(edge.to_node_id)?.add(edge.from_node_id)
  }
  const visited = new Set<string>()
  const components: string[][] = []
  for (const node of nodes) {
    if (visited.has(node.id)) {
      continue
    }
    const component: string[] = []
    const queue = [node.id]
    visited.add(node.id)
    while (queue.length > 0) {
      const current = queue.shift()!
      component.push(current)
      for (const neighbor of adjacency.get(current) ?? []) {
        if (visited.has(neighbor)) {
          continue
        }
        visited.add(neighbor)
        queue.push(neighbor)
      }
    }
    components.push(component)
  }
  return components
}

function assignNodeRanks(
  nodes: WorkflowNodeDefinition[],
  edges: WorkflowEdgeDefinition[],
  endpoints: WorkflowEndpointDefinition[],
) {
  const nodeIds = nodes.map((node) => node.id)
  const outgoing = new Map<string, string[]>()
  const incomingCount = new Map<string, number>(nodeIds.map((nodeId) => [nodeId, 0]))
  for (const nodeId of nodeIds) {
    outgoing.set(nodeId, [])
  }
  for (const edge of edges) {
    outgoing.get(edge.from_node_id)?.push(edge.to_node_id)
    incomingCount.set(edge.to_node_id, (incomingCount.get(edge.to_node_id) ?? 0) + 1)
  }
  const roots = uniquePreservingOrder([
    ...endpoints.map((endpoint) => endpoint.entry_node_id),
    ...nodes.filter((node) => (incomingCount.get(node.id) ?? 0) === 0).map((node) => node.id),
    ...nodeIds,
  ])
  const rankById = new Map<string, number>()
  const queue = roots.map((nodeId) => ({ nodeId, rank: 0 }))
  let safety = nodeIds.length * Math.max(1, edges.length + 1) * 2
  while (queue.length > 0 && safety > 0) {
    safety -= 1
    const current = queue.shift()!
    const previous = rankById.get(current.nodeId)
    if (previous !== undefined && previous >= current.rank) {
      continue
    }
    rankById.set(current.nodeId, current.rank)
    for (const nextNodeId of outgoing.get(current.nodeId) ?? []) {
      queue.push({ nodeId: nextNodeId, rank: current.rank + 1 })
    }
  }
  for (const nodeId of nodeIds) {
    if (!rankById.has(nodeId)) {
      rankById.set(nodeId, 0)
    }
  }
  return rankById
}

function groupNodesByRank(
  nodes: WorkflowNodeDefinition[],
  rankById: Map<string, number>,
) {
  const levels = new Map<number, WorkflowNodeDefinition[]>()
  for (const node of nodes) {
    const rank = rankById.get(node.id) ?? 0
    const levelNodes = levels.get(rank) ?? []
    levelNodes.push(node)
    levels.set(rank, levelNodes)
  }
  return [...levels.entries()]
    .sort((left, right) => left[0] - right[0])
    .map((entry) => entry[1].sort((left, right) => left.id.localeCompare(right.id)))
}

function formatNodeLines(
  options: {
    title: string
    provider: string | null
    model: string | null
    effort: string | null
  },
  width: number,
) {
  const innerWidth = Math.max(4, width - 2)
  return [
    truncateLine(options.title, innerWidth),
    truncateLine(options.provider ? `provider ${options.provider}` : "provider -", innerWidth),
    truncateLine(options.model ? `model ${options.model}` : "model -", innerWidth),
    truncateLine(options.effort ? `effort ${options.effort}` : "effort -", innerWidth),
  ]
}

function truncateLine(value: string, width: number) {
  if (value.length <= width) {
    return value
  }
  if (width <= 3) {
    return value.slice(0, width)
  }
  return `${value.slice(0, width - 3)}...`
}

function uniquePreservingOrder(values: string[]) {
  return values.filter((value, index) => {
    return value.length > 0 && values.indexOf(value) === index
  })
}
