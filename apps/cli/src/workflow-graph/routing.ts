import type { WorkflowGraphNodeLayout } from "./types.js"

type EdgeAnchorSide = "top" | "right" | "bottom" | "left"
type ReciprocalLane = -1 | 1
type Point = { x: number; y: number }
type Direction = "up" | "right" | "down" | "left"
type EdgeAnchor = Point & { side: EdgeAnchorSide }
type RoutedPath = {
  points: Point[]
  score: number
}

const SEARCH_PADDING = 6
const TURN_PENALTY = 10
const OBSTACLE_ADJACENCY_PENALTY = 6

export function routeWorkflowEdge(
  fromNode: WorkflowGraphNodeLayout,
  toNode: WorkflowGraphNodeLayout,
  options?: {
    reciprocalLane?: ReciprocalLane | null
    obstacles?: WorkflowGraphNodeLayout[]
  },
) {
  const laneOffset = options?.reciprocalLane ?? 0
  const obstacles = options?.obstacles ?? [fromNode, toNode]
  const fromAnchors = borderCenterAnchors(fromNode, laneOffset)
  const toAnchors = borderCenterAnchors(toNode, laneOffset)
  let bestRoute: RoutedPath | null = null

  for (const from of fromAnchors) {
    for (const to of toAnchors) {
      const route = routeBetweenAnchors(from, to, obstacles)
      if (!route) {
        continue
      }
      const facingPenalty = anchorFacingPenalty(from, to) + anchorFacingPenalty(to, from)
      const straightPenalty = from.x === to.x || from.y === to.y ? 0 : 3
      const score = route.score + facingPenalty * 8 + straightPenalty
      if (!bestRoute || score < bestRoute.score) {
        bestRoute = { points: route.points, score }
      }
    }
  }

  if (bestRoute) {
    return bestRoute.points
  }

  const fallbackPair = chooseNearestAnchorPair(fromAnchors, toAnchors)
  if (options?.reciprocalLane) {
    return buildReciprocalPath(fallbackPair.from, fallbackPair.to, options.reciprocalLane)
  }
  return buildOrthogonalFallbackPath(fallbackPair.from, fallbackPair.to)
}

function borderCenterAnchors(node: WorkflowGraphNodeLayout, laneOffset: number): EdgeAnchor[] {
  const centerX = node.x + Math.floor(node.width / 2)
  const centerY = node.y + Math.floor(node.height / 2)
  const minInteriorX = node.x + 1
  const maxInteriorX = node.x + node.width - 2
  const minInteriorY = node.y + 1
  const maxInteriorY = node.y + node.height - 2
  const clampX = (value: number) => clamp(value, minInteriorX, maxInteriorX)
  const clampY = (value: number) => clamp(value, minInteriorY, maxInteriorY)
  return [
    { side: "top", x: clampX(centerX + laneOffset), y: node.y },
    { side: "right", x: node.x + node.width - 1, y: clampY(centerY + laneOffset) },
    { side: "bottom", x: clampX(centerX + laneOffset), y: node.y + node.height - 1 },
    { side: "left", x: node.x, y: clampY(centerY + laneOffset) },
  ]
}

function routeBetweenAnchors(
  from: EdgeAnchor,
  to: EdgeAnchor,
  obstacles: WorkflowGraphNodeLayout[],
): RoutedPath | null {
  const start = stepOutsideAnchor(from)
  const goal = stepOutsideAnchor(to)
  if (isBlockedCell(start, obstacles) || isBlockedCell(goal, obstacles)) {
    return null
  }
  const cells = findOrthogonalRoute(start, goal, obstacles)
  if (!cells) {
    return null
  }
  const polyline = compressCellsToPolyline(cells)
  const points = compressPolyline([{ x: from.x, y: from.y }, ...polyline, { x: to.x, y: to.y }])
  return {
    points,
    score: routePolylineScore(points),
  }
}

function findOrthogonalRoute(start: Point, goal: Point, obstacles: WorkflowGraphNodeLayout[]) {
  const blocked = buildBlockedCellSet(obstacles)
  const minObstacleX = Math.min(start.x, goal.x, ...obstacles.map((node) => node.x)) - SEARCH_PADDING
  const maxObstacleX = Math.max(start.x, goal.x, ...obstacles.map((node) => node.x + node.width - 1)) + SEARCH_PADDING
  const minObstacleY = Math.min(start.y, goal.y, ...obstacles.map((node) => node.y)) - SEARCH_PADDING
  const maxObstacleY = Math.max(start.y, goal.y, ...obstacles.map((node) => node.y + node.height - 1)) + SEARCH_PADDING
  const minX = Math.max(0, minObstacleX)
  const minY = Math.max(0, minObstacleY)
  const maxX = Math.max(goal.x, maxObstacleX)
  const maxY = Math.max(goal.y, maxObstacleY)

  type SearchState = {
    x: number
    y: number
    direction: Direction | null
    cost: number
    priority: number
  }

  const frontier: SearchState[] = [{
    x: start.x,
    y: start.y,
    direction: null,
    cost: 0,
    priority: manhattanDistance(start, goal),
  }]
  const bestCost = new Map<string, number>([[searchKey(start.x, start.y, null), 0]])
  const previous = new Map<string, string | null>([[searchKey(start.x, start.y, null), null]])

  while (frontier.length > 0) {
    frontier.sort((left, right) => left.priority - right.priority)
    const current = frontier.shift()!
    if (current.x === goal.x && current.y === goal.y) {
      return reconstructCells(previous, current)
    }

    for (const neighbor of neighboringStates(current)) {
      if (neighbor.x < minX || neighbor.x > maxX || neighbor.y < minY || neighbor.y > maxY) {
        continue
      }
      if (blocked.has(cellKey(neighbor.x, neighbor.y))) {
        continue
      }
      const nextCost = current.cost
        + 1
        + (current.direction && current.direction !== neighbor.direction ? TURN_PENALTY : 0)
        + adjacencyPenalty(neighbor, blocked)
      const key = searchKey(neighbor.x, neighbor.y, neighbor.direction)
      const seenCost = bestCost.get(key)
      if (seenCost !== undefined && seenCost <= nextCost) {
        continue
      }
      bestCost.set(key, nextCost)
      previous.set(key, searchKey(current.x, current.y, current.direction))
      frontier.push({
        ...neighbor,
        cost: nextCost,
        priority: nextCost + manhattanDistance(neighbor, goal),
      })
    }
  }

  return null
}

function neighboringStates(state: { x: number; y: number }) {
  return [
    { x: state.x, y: state.y - 1, direction: "up" as const },
    { x: state.x + 1, y: state.y, direction: "right" as const },
    { x: state.x, y: state.y + 1, direction: "down" as const },
    { x: state.x - 1, y: state.y, direction: "left" as const },
  ]
}

function reconstructCells(previous: Map<string, string | null>, state: { x: number; y: number; direction: Direction | null }) {
  const cells: Point[] = []
  let key: string | null = searchKey(state.x, state.y, state.direction)
  while (key) {
    const [xToken, yToken] = key.split("|", 2)
    const x = Number.parseInt(xToken ?? "0", 10)
    const y = Number.parseInt(yToken ?? "0", 10)
    cells.push({ x, y })
    key = previous.get(key) ?? null
  }
  return cells.reverse()
}

function compressCellsToPolyline(cells: Point[]) {
  if (cells.length <= 2) {
    return cells
  }
  const points = [cells[0]!]
  for (let index = 1; index < cells.length - 1; index += 1) {
    const previous = cells[index - 1]!
    const current = cells[index]!
    const next = cells[index + 1]!
    const horizontalBefore = previous.y === current.y
    const horizontalAfter = current.y === next.y
    if (horizontalBefore !== horizontalAfter) {
      points.push(current)
    }
  }
  points.push(cells[cells.length - 1]!)
  return points
}

function routePolylineScore(points: Point[]) {
  let score = 0
  for (let index = 0; index < points.length - 1; index += 1) {
    const current = points[index]!
    const next = points[index + 1]!
    score += manhattanDistance(current, next)
    if (index > 0) {
      score += 2
    }
  }
  return score
}

function adjacencyPenalty(point: Point, blocked: Set<string>) {
  let penalty = 0
  for (const neighbor of neighboringStates(point)) {
    if (blocked.has(cellKey(neighbor.x, neighbor.y))) {
      penalty += OBSTACLE_ADJACENCY_PENALTY
    }
  }
  return penalty
}

function buildBlockedCellSet(obstacles: WorkflowGraphNodeLayout[]) {
  const blocked = new Set<string>()
  for (const node of obstacles) {
    for (let x = node.x; x < node.x + node.width; x += 1) {
      for (let y = node.y; y < node.y + node.height; y += 1) {
        blocked.add(cellKey(x, y))
      }
    }
  }
  return blocked
}

function isBlockedCell(point: Point, obstacles: WorkflowGraphNodeLayout[]) {
  return obstacles.some((node) => (
    point.x >= node.x
    && point.x < node.x + node.width
    && point.y >= node.y
    && point.y < node.y + node.height
  ))
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

function buildOrthogonalFallbackPath(from: EdgeAnchor, to: EdgeAnchor) {
  if (from.x === to.x || from.y === to.y) {
    return [{ x: from.x, y: from.y }, { x: to.x, y: to.y }]
  }
  return dedupeAdjacentPoints([
    { x: from.x, y: from.y },
    stepOutsideAnchor(from),
    { x: stepOutsideAnchor(from).x, y: stepOutsideAnchor(to).y },
    stepOutsideAnchor(to),
    { x: to.x, y: to.y },
  ])
}

function buildReciprocalPath(
  from: EdgeAnchor,
  to: EdgeAnchor,
  reciprocalLane: ReciprocalLane,
) {
  const fromOutward = stepOutsideAnchor(from)
  const toOutward = stepOutsideAnchor(to)

  if (Math.abs(to.x - from.x) >= Math.abs(to.y - from.y)) {
    const laneY = fromOutward.y + reciprocalLane * 2
    return dedupeAdjacentPoints([
      { x: from.x, y: from.y },
      fromOutward,
      { x: fromOutward.x, y: laneY },
      { x: toOutward.x, y: laneY },
      toOutward,
      { x: to.x, y: to.y },
    ])
  }

  const laneX = fromOutward.x + reciprocalLane * 2
  return dedupeAdjacentPoints([
    { x: from.x, y: from.y },
    fromOutward,
    { x: laneX, y: fromOutward.y },
    { x: laneX, y: toOutward.y },
    toOutward,
    { x: to.x, y: to.y },
  ])
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

function dedupeAdjacentPoints(points: Point[]) {
  const deduped: Point[] = []
  for (const point of points) {
    const previous = deduped[deduped.length - 1]
    if (previous && previous.x === point.x && previous.y === point.y) {
      continue
    }
    deduped.push(point)
  }
  return deduped
}

function compressPolyline(points: Point[]) {
  const deduped = dedupeAdjacentPoints(points)
  if (deduped.length <= 2) {
    return deduped
  }
  const compressed = [deduped[0]!]
  for (let index = 1; index < deduped.length - 1; index += 1) {
    const previous = deduped[index - 1]!
    const current = deduped[index]!
    const next = deduped[index + 1]!
    const directionBefore = directionBetween(previous, current)
    const directionAfter = directionBetween(current, next)
    if (directionBefore !== directionAfter) {
      compressed.push(current)
    }
  }
  compressed.push(deduped[deduped.length - 1]!)
  return compressed
}

function directionBetween(from: Point, to: Point) {
  if (from.x === to.x) {
    return "vertical"
  }
  if (from.y === to.y) {
    return "horizontal"
  }
  return "corner"
}

function manhattanDistance(from: Point, to: Point) {
  return Math.abs(to.x - from.x) + Math.abs(to.y - from.y)
}

function cellKey(x: number, y: number) {
  return `${x}:${y}`
}

function searchKey(x: number, y: number, direction: Direction | null) {
  return `${x}|${y}|${direction ?? "start"}`
}

function clamp(value: number, min: number, max: number) {
  if (min > max) {
    return value
  }
  return Math.max(min, Math.min(max, value))
}
