import type { AgentInstance, WorkflowDefinition, WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowNodeDefinition } from "./cli-types.js"

export const WORKFLOW_ZOOM_LEVELS = [0.8, 1, 1.25, 1.5, 1.8] as const
export const DEFAULT_WORKFLOW_ZOOM_INDEX = 1

const BASE_NODE_WIDTH = 30
const BASE_NODE_HEIGHT = 8
const BASE_HORIZONTAL_GAP = 8
const BASE_VERTICAL_GAP = 5
const BASE_COMPONENT_GAP = 7
const GRAPH_PADDING_X = 4
const GRAPH_PADDING_Y = 3
const MIN_NODE_WIDTH = 24
const MIN_NODE_HEIGHT = 7

export type WorkflowGraphMetrics = {
  scale: number
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

export function resolveWorkflowZoomMetrics(zoomIndex: number): WorkflowGraphMetrics {
  const clampedIndex = Math.max(0, Math.min(zoomIndex, WORKFLOW_ZOOM_LEVELS.length - 1))
  const scale = WORKFLOW_ZOOM_LEVELS[clampedIndex]!
  return {
    scale,
    nodeWidth: Math.max(MIN_NODE_WIDTH, Math.round(BASE_NODE_WIDTH * scale)),
    nodeHeight: Math.max(MIN_NODE_HEIGHT, Math.round(BASE_NODE_HEIGHT * scale)),
    horizontalGap: Math.max(4, Math.round(BASE_HORIZONTAL_GAP * scale)),
    verticalGap: Math.max(3, Math.round(BASE_VERTICAL_GAP * scale)),
    componentGap: Math.max(4, Math.round(BASE_COMPONENT_GAP * scale)),
    endpointGap: Math.max(2, Math.round(3 * scale)),
  }
}

export function resolveSelectedWorkflow(
  workflows: WorkflowDefinition[],
  selectedWorkflowId: string | null,
): WorkflowDefinition | null {
  if (workflows.length === 0) {
    return null
  }
  if (selectedWorkflowId) {
    const exact = workflows.find((workflow) => workflow.id === selectedWorkflowId)
    if (exact) {
      return exact
    }
  }
  return workflows[0] ?? null
}

export function resolveSelectedWorkflowNodeId(
  workflow: WorkflowDefinition | null,
  selectedNodeId: string | null,
): string | null {
  const nodes = workflow?.nodes ?? []
  if (nodes.length === 0) {
    return null
  }
  if (selectedNodeId && nodes.some((node) => node.id === selectedNodeId)) {
    return selectedNodeId
  }
  return nodes[0]?.id ?? null
}

export function cycleWorkflowNodeId(
  workflow: WorkflowDefinition | null,
  selectedNodeId: string | null,
  step = 1,
): string | null {
  const nodes = workflow?.nodes ?? []
  if (nodes.length === 0) {
    return null
  }
  if (!selectedNodeId) {
    return nodes[0]?.id ?? null
  }
  const currentIndex = nodes.findIndex((node) => node.id === selectedNodeId)
  if (currentIndex < 0) {
    return nodes[0]?.id ?? null
  }
  const nextIndex = modulo(currentIndex + step, nodes.length)
  return nodes[nextIndex]?.id ?? null
}

export function deriveWorkflowZoomIndex(currentZoomIndex: number, direction: "in" | "out") {
  return Math.max(
    0,
    Math.min(
      WORKFLOW_ZOOM_LEVELS.length - 1,
      currentZoomIndex + (direction === "in" ? 1 : -1),
    ),
  )
}

export function derivePointerAnchoredViewport(options: {
  viewportWidth: number
  viewportHeight: number
  pointerX: number
  pointerY: number
  scrollLeft: number
  scrollTop: number
  previousContentWidth: number
  previousContentHeight: number
  nextContentWidth: number
  nextContentHeight: number
}) {
  const clampedPointerX = clamp(options.pointerX, 0, Math.max(0, options.viewportWidth - 1))
  const clampedPointerY = clamp(options.pointerY, 0, Math.max(0, options.viewportHeight - 1))
  const anchorRatioX = options.previousContentWidth <= 0
    ? 0
    : (options.scrollLeft + clampedPointerX) / Math.max(1, options.previousContentWidth)
  const anchorRatioY = options.previousContentHeight <= 0
    ? 0
    : (options.scrollTop + clampedPointerY) / Math.max(1, options.previousContentHeight)
  const nextScrollLeft = Math.round(anchorRatioX * options.nextContentWidth - clampedPointerX)
  const nextScrollTop = Math.round(anchorRatioY * options.nextContentHeight - clampedPointerY)
  return {
    x: clamp(nextScrollLeft, 0, Math.max(0, options.nextContentWidth - options.viewportWidth)),
    y: clamp(nextScrollTop, 0, Math.max(0, options.nextContentHeight - options.viewportHeight)),
  }
}

export function buildWorkflowGraphLayout(options: {
  workflow: WorkflowDefinition
  agents: AgentInstance[]
  selectedNodeId: string | null
  zoomIndex: number
}): WorkflowGraphLayout {
  const metrics = resolveWorkflowZoomMetrics(options.zoomIndex)
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
        points: routeEdge(fromNode, toNode),
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

function routeEdge(fromNode: WorkflowGraphNodeLayout, toNode: WorkflowGraphNodeLayout) {
  const fromX = fromNode.x + Math.floor(fromNode.width / 2)
  const fromY = fromNode.y + fromNode.height - 1
  const toX = toNode.x + Math.floor(toNode.width / 2)
  const toY = toNode.y - 1
  const midY = fromY + Math.max(1, Math.floor((toY - fromY) / 2))
  return [
    { x: fromX, y: fromY },
    { x: fromX, y: midY },
    { x: toX, y: midY },
    { x: toX, y: toY },
  ]
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
  const seen = new Set<string>()
  return values.filter((value) => {
    if (seen.has(value)) {
      return false
    }
    seen.add(value)
    return true
  })
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value))
}

function modulo(value: number, divisor: number) {
  return ((value % divisor) + divisor) % divisor
}
