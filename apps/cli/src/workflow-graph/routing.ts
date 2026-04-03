import type { WorkflowGraphNodeLayout } from "./types.js"

type EdgeAnchorSide = "top" | "right" | "bottom" | "left"
type EdgeAnchor = { x: number; y: number; side: EdgeAnchorSide }
type ReciprocalLane = -1 | 1

export function routeWorkflowEdge(
  fromNode: WorkflowGraphNodeLayout,
  toNode: WorkflowGraphNodeLayout,
  options?: { reciprocalLane?: ReciprocalLane | null },
) {
  const fromAnchors = borderCenterAnchors(fromNode)
  const toAnchors = borderCenterAnchors(toNode)
  const chosenPair = chooseNearestAnchorPair(fromAnchors, toAnchors)
  if (options?.reciprocalLane) {
    return buildReciprocalPath(chosenPair.from, chosenPair.to, options.reciprocalLane)
  }
  return buildOrthogonalPath(chosenPair.from, chosenPair.to, fromNode, toNode)
}

function borderCenterAnchors(node: WorkflowGraphNodeLayout): EdgeAnchor[] {
  const centerX = node.x + Math.floor(node.width / 2)
  const centerY = node.y + Math.floor(node.height / 2)
  return [
    { side: "top", x: centerX, y: node.y },
    { side: "right", x: node.x + node.width - 1, y: centerY },
    { side: "bottom", x: centerX, y: node.y + node.height - 1 },
    { side: "left", x: node.x, y: centerY },
  ]
}

function chooseNearestAnchorPair(fromAnchors: EdgeAnchor[], toAnchors: EdgeAnchor[]) {
  let bestPair = { from: fromAnchors[0]!, to: toAnchors[0]!, score: Number.POSITIVE_INFINITY }
  for (const from of fromAnchors) {
    for (const to of toAnchors) {
      const manhattan = Math.abs(from.x - to.x) + Math.abs(from.y - to.y)
      const nonLinearPenalty = from.x === to.x || from.y === to.y ? 0 : 1
      const fromFacingPenalty = anchorFacingPenalty(from, to)
      const toFacingPenalty = anchorFacingPenalty(to, from)
      const score = manhattan * 16 + nonLinearPenalty * 4 + fromFacingPenalty + toFacingPenalty
      if (score < bestPair.score) {
        bestPair = { from, to, score }
      }
    }
  }
  return bestPair
}

function anchorFacingPenalty(from: EdgeAnchor, to: EdgeAnchor) {
  const dx = to.x - from.x
  const dy = to.y - from.y
  switch (from.side) {
    case "top":
      return dy <= 0 ? 0 : 1
    case "right":
      return dx >= 0 ? 0 : 1
    case "bottom":
      return dy >= 0 ? 0 : 1
    case "left":
      return dx <= 0 ? 0 : 1
    default:
      return 0
  }
}

function buildOrthogonalPath(
  from: EdgeAnchor,
  to: EdgeAnchor,
  fromNode: WorkflowGraphNodeLayout,
  toNode: WorkflowGraphNodeLayout,
) {
  if (from.x === to.x || from.y === to.y) {
    return [{ x: from.x, y: from.y }, { x: to.x, y: to.y }]
  }

  const elbowA = { x: from.x, y: to.y }
  const elbowB = { x: to.x, y: from.y }
  const pathA = [{ x: from.x, y: from.y }, elbowA, { x: to.x, y: to.y }]
  const pathB = [{ x: from.x, y: from.y }, elbowB, { x: to.x, y: to.y }]
  const scoreA = orthogonalPathScore(pathA, fromNode, toNode)
  const scoreB = orthogonalPathScore(pathB, fromNode, toNode)
  return dedupeAdjacentPoints(scoreA <= scoreB ? pathA : pathB)
}

function buildReciprocalPath(
  from: EdgeAnchor,
  to: EdgeAnchor,
  reciprocalLane: ReciprocalLane,
) {
  const fromOutward = stepOutsideAnchor(from)
  const toOutward = stepOutsideAnchor(to)

  if (Math.abs(to.x - from.x) >= Math.abs(to.y - from.y)) {
    const laneY = from.y + reciprocalLane * 2
    return dedupeAdjacentPoints([
      { x: from.x, y: from.y },
      fromOutward,
      { x: fromOutward.x, y: laneY },
      { x: toOutward.x, y: laneY },
      toOutward,
      { x: to.x, y: to.y },
    ])
  }

  const laneX = from.x + reciprocalLane * 2
  return dedupeAdjacentPoints([
    { x: from.x, y: from.y },
    fromOutward,
    { x: laneX, y: fromOutward.y },
    { x: laneX, y: toOutward.y },
    toOutward,
    { x: to.x, y: to.y },
  ])
}

function orthogonalPathScore(
  path: Array<{ x: number; y: number }>,
  fromNode: WorkflowGraphNodeLayout,
  toNode: WorkflowGraphNodeLayout,
) {
  let score = 0
  for (let index = 0; index < path.length - 1; index += 1) {
    const start = path[index]!
    const end = path[index + 1]!
    score += Math.abs(end.x - start.x) + Math.abs(end.y - start.y)
    if (segmentOverlapsNode(start, end, fromNode)) {
      score += 10
    }
    if (segmentOverlapsNode(start, end, toNode)) {
      score += 10
    }
  }
  return score
}

function segmentOverlapsNode(
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

function dedupeAdjacentPoints(points: Array<{ x: number; y: number }>) {
  const deduped: Array<{ x: number; y: number }> = []
  for (const point of points) {
    const previous = deduped[deduped.length - 1]
    if (previous && previous.x === point.x && previous.y === point.y) {
      continue
    }
    deduped.push(point)
  }
  return deduped
}

function stepOutsideAnchor(anchor: EdgeAnchor) {
  switch (anchor.side) {
    case "top":
      return { x: anchor.x, y: anchor.y - 1 }
    case "right":
      return { x: anchor.x + 1, y: anchor.y }
    case "bottom":
      return { x: anchor.x, y: anchor.y + 1 }
    case "left":
      return { x: anchor.x - 1, y: anchor.y }
    default:
      return { x: anchor.x, y: anchor.y }
  }
}
