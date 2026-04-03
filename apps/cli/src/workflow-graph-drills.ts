import type { AgentInstance, WorkflowDefinition } from "./cli-types.js"
import { buildWorkflowEdgeCells } from "./workflow-graph/render.js"
import type { WorkflowGraphLayout, WorkflowGraphNodeLayout } from "./workflow-graph/types.js"

export type WorkflowGraphDrillCase = {
  id: string
  title: string
  workflow: WorkflowDefinition
  agents: AgentInstance[]
}

export type WorkflowGraphLayoutValidation = {
  nodeOverlaps: Array<{ leftNodeId: string; rightNodeId: string }>
  diagonalSegments: Array<{ edgeId: string; fromNodeId: string; toNodeId: string }>
  edgeNodeCollisions: Array<{
    edgeId: string
    fromNodeId: string
    toNodeId: string
    nodeId: string
    segmentStart: { x: number; y: number }
    segmentEnd: { x: number; y: number }
  }>
  reciprocalOverlaps: Array<{ forwardEdgeId: string; reverseEdgeId: string }>
}

export function buildWorkflowGraphDrillCases(): WorkflowGraphDrillCase[] {
  return [
    drillCase("linear-chain", "Linear Chain", ["a", "b", "c", "d", "e", "f"], [
      ["a", "b"],
      ["b", "c"],
      ["c", "d"],
      ["d", "e"],
      ["e", "f"],
    ]),
    drillCase("wide-fan-out", "Wide Fan Out", ["root", "a", "b", "c", "d", "e"], [
      ["root", "a"],
      ["root", "b"],
      ["root", "c"],
      ["root", "d"],
      ["root", "e"],
    ]),
    drillCase("wide-fan-in", "Wide Fan In", ["a", "b", "c", "d", "e", "sink"], [
      ["a", "sink"],
      ["b", "sink"],
      ["c", "sink"],
      ["d", "sink"],
      ["e", "sink"],
    ]),
    drillCase("diamond-join", "Diamond Join", ["root", "left", "right", "join", "out"], [
      ["root", "left"],
      ["root", "right"],
      ["left", "join"],
      ["right", "join"],
      ["join", "out"],
    ]),
    drillCase("reciprocal-pair", "Reciprocal Pair", ["a", "b"], [
      ["a", "b"],
      ["b", "a"],
    ]),
    drillCase("reciprocal-branch-join", "Reciprocal Branch Join", ["a", "b", "c", "d"], [
      ["a", "b"],
      ["b", "a"],
      ["a", "c"],
      ["b", "d"],
      ["c", "d"],
    ]),
    drillCase("crossing-order", "Crossing Order", ["root", "a", "b", "c", "d", "e"], [
      ["root", "a"],
      ["root", "b"],
      ["a", "d"],
      ["b", "c"],
      ["a", "e"],
      ["b", "d"],
    ]),
    drillCase("multi-parent-multi-child", "Multi Parent Multi Child", ["a", "b", "c", "d", "e", "f"], [
      ["a", "c"],
      ["a", "d"],
      ["b", "d"],
      ["b", "e"],
      ["c", "f"],
      ["d", "f"],
      ["e", "f"],
    ]),
    drillCase("cycle-square", "Cycle Square", ["a", "b", "c", "d"], [
      ["a", "b"],
      ["b", "c"],
      ["c", "d"],
      ["d", "a"],
    ]),
    drillCase("two-components", "Two Components", ["a", "b", "c", "d", "e"], [
      ["a", "b"],
      ["c", "d"],
      ["d", "e"],
    ]),
    drillCase("mixed-eight", "Mixed Eight", ["root", "a", "b", "c", "d", "e", "f", "g"], [
      ["root", "a"],
      ["root", "b"],
      ["root", "c"],
      ["a", "d"],
      ["a", "e"],
      ["b", "e"],
      ["b", "f"],
      ["c", "f"],
      ["d", "g"],
      ["e", "g"],
      ["f", "g"],
    ]),
  ]
}

export function validateWorkflowGraphLayout(layout: WorkflowGraphLayout): WorkflowGraphLayoutValidation {
  const nodeOverlaps: WorkflowGraphLayoutValidation["nodeOverlaps"] = []
  const diagonalSegments: WorkflowGraphLayoutValidation["diagonalSegments"] = []
  const edgeNodeCollisions: WorkflowGraphLayoutValidation["edgeNodeCollisions"] = []
  const reciprocalOverlaps: WorkflowGraphLayoutValidation["reciprocalOverlaps"] = []

  for (let index = 0; index < layout.nodes.length; index += 1) {
    const leftNode = layout.nodes[index]!
    for (let otherIndex = index + 1; otherIndex < layout.nodes.length; otherIndex += 1) {
      const rightNode = layout.nodes[otherIndex]!
      if (!nodesOverlap(leftNode, rightNode)) {
        continue
      }
      nodeOverlaps.push({ leftNodeId: leftNode.id, rightNodeId: rightNode.id })
    }
  }

  for (const edge of layout.edges) {
    const fromNode = layout.nodes.find((node) => node.id === edge.fromNodeId)
    const toNode = layout.nodes.find((node) => node.id === edge.toNodeId)
    if (!fromNode || !toNode) {
      continue
    }
    for (let pointIndex = 0; pointIndex < edge.points.length - 1; pointIndex += 1) {
      const start = edge.points[pointIndex]!
      const end = edge.points[pointIndex + 1]!
      if (!(start.x === end.x || start.y === end.y)) {
        diagonalSegments.push({
          edgeId: edge.id,
          fromNodeId: edge.fromNodeId,
          toNodeId: edge.toNodeId,
        })
        continue
      }
      for (const node of layout.nodes) {
        if (node.id === fromNode.id || node.id === toNode.id) {
          continue
        }
        if (!segmentIntersectsNode(start, end, node)) {
          continue
        }
        edgeNodeCollisions.push({
          edgeId: edge.id,
          fromNodeId: edge.fromNodeId,
          toNodeId: edge.toNodeId,
          nodeId: node.id,
          segmentStart: start,
          segmentEnd: end,
        })
      }
    }
  }

  for (const edge of layout.edges) {
    const reverse = layout.edges.find((other) => other.fromNodeId === edge.toNodeId && other.toNodeId === edge.fromNodeId)
    if (!reverse || edge.id >= reverse.id) {
      continue
    }
    if (JSON.stringify(edge.points) !== JSON.stringify(reverse.points)) {
      continue
    }
    reciprocalOverlaps.push({ forwardEdgeId: edge.id, reverseEdgeId: reverse.id })
  }

  return {
    nodeOverlaps,
    diagonalSegments,
    edgeNodeCollisions,
    reciprocalOverlaps,
  }
}

export function renderWorkflowGraphAscii(layout: WorkflowGraphLayout) {
  const width = Math.max(1, layout.width + 1)
  const height = Math.max(1, layout.height + 1)
  const grid = Array.from({ length: height }, () => Array.from({ length: width }, () => " "))

  for (const endpoint of layout.endpoints) {
    setCell(grid, endpoint.markerX, endpoint.markerY, "o")
    if (endpoint.markerY + 1 < height) {
      setCell(grid, endpoint.markerX, endpoint.markerY + 1, "|")
    }
    for (let index = 0; index < endpoint.label.length; index += 1) {
      setCell(grid, endpoint.labelX + index, endpoint.labelY, endpoint.label[index]!)
    }
  }

  for (const edge of layout.edges) {
    for (const cell of buildWorkflowEdgeCells(edge.points)) {
      setCell(grid, cell.x, cell.y, cell.char)
    }
  }

  for (const node of layout.nodes) {
    const minX = node.x
    const maxX = node.x + node.width - 1
    const minY = node.y
    const maxY = node.y + node.height - 1
    for (let x = minX; x <= maxX; x += 1) {
      setCell(grid, x, minY, "-")
      setCell(grid, x, maxY, "-")
    }
    for (let y = minY; y <= maxY; y += 1) {
      setCell(grid, minX, y, "|")
      setCell(grid, maxX, y, "|")
    }
    setCell(grid, minX, minY, "+")
    setCell(grid, maxX, minY, "+")
    setCell(grid, minX, maxY, "+")
    setCell(grid, maxX, maxY, "+")
    const label = node.id.startsWith("node-") ? node.id.slice(5) : node.id
    for (let index = 0; index < label.length && minX + 2 + index < maxX; index += 1) {
      setCell(grid, minX + 2 + index, minY + 1, label[index]!)
    }
  }

  return grid
    .map((row) => row.join("").replace(/\s+$/u, ""))
    .join("\n")
}

function drillCase(
  id: string,
  title: string,
  nodeNames: string[],
  edges: Array<[string, string]>,
): WorkflowGraphDrillCase {
  const nodes = nodeNames.map((nodeName, index) => ({
    id: `node-${nodeName}`,
    agent_id: `agent-${index + 1}`,
  }))
  return {
    id,
    title,
    workflow: {
      id: `workflow-${id}`,
      alias: title.toLowerCase(),
      nodes,
      edges: edges.map(([fromNodeName, toNodeName], index) => ({
        id: `edge-${index + 1}`,
        from_node_id: `node-${fromNodeName}`,
        to_node_id: `node-${toNodeName}`,
      })),
      endpoints: [
        {
          id: `endpoint-${id}`,
          alias: "start",
          entry_node_id: nodes[0]!.id,
        },
      ],
    },
    agents: nodes.map((node, index) => drillAgent(node.agent_id, index)),
  }
}

function drillAgent(id: string, index: number): AgentInstance {
  const aliases = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]
  const efforts = [null, "high", "medium", "low"] as const
  const effort = efforts[index % efforts.length] ?? null
  return {
    id,
    agent_ref: id.replace("agent-", "ag-"),
    session_id: "session-drill",
    alias: aliases[index] ?? null,
    provider: "opencode",
    model: "openai/gpt-5.4",
    effort,
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: Math.floor(index / 2),
    grid_col: index % 2,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: index,
    last_activity_at_ms: index,
  }
}

function nodesOverlap(leftNode: WorkflowGraphNodeLayout, rightNode: WorkflowGraphNodeLayout) {
  return (
    leftNode.x < rightNode.x + rightNode.width
    && leftNode.x + leftNode.width > rightNode.x
    && leftNode.y < rightNode.y + rightNode.height
    && leftNode.y + leftNode.height > rightNode.y
  )
}

function segmentIntersectsNode(
  start: { x: number; y: number },
  end: { x: number; y: number },
  node: WorkflowGraphNodeLayout,
) {
  const minX = node.x
  const maxX = node.x + node.width - 1
  const minY = node.y
  const maxY = node.y + node.height - 1
  if (start.x === end.x) {
    if (start.x < minX || start.x > maxX) {
      return false
    }
    const segmentMinY = Math.min(start.y, end.y)
    const segmentMaxY = Math.max(start.y, end.y)
    return segmentMaxY >= minY && segmentMinY <= maxY
  }
  if (start.y === end.y) {
    if (start.y < minY || start.y > maxY) {
      return false
    }
    const segmentMinX = Math.min(start.x, end.x)
    const segmentMaxX = Math.max(start.x, end.x)
    return segmentMaxX >= minX && segmentMinX <= maxX
  }
  return false
}

function setCell(grid: string[][], x: number, y: number, value: string) {
  const row = grid[y]
  if (!row) {
    return
  }
  if (x < 0 || x >= row.length) {
    return
  }
  row[x] = value
}
