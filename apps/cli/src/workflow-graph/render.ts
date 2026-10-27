export function buildWorkflowEdgeCells(points: Array<{ x: number; y: number }>) {
  const cells = new Map<string, { x: number; y: number; char: string }>()
  if (points.length < 2) {
    return []
  }
  for (let index = 0; index < points.length - 1; index += 1) {
    const current = points[index]!
    const next = points[index + 1]!
    const lastSegment = index === points.length - 2
    if (current.x === next.x) {
      const step = current.y <= next.y ? 1 : -1
      for (let y = current.y; ; y += step) {
        const atEnd = y === next.y
        const char = lastSegment && atEnd
          ? (step > 0 ? "v" : "^")
          : (isTurnPoint(points, index + 1, current.x, y) ? "+" : "|")
        cells.set(`${current.x}:${y}`, { x: current.x, y, char })
        if (atEnd) {
          break
        }
      }
      continue
    }
    if (current.y === next.y) {
      const step = current.x <= next.x ? 1 : -1
      for (let x = current.x; ; x += step) {
        const atEnd = x === next.x
        const char = lastSegment && atEnd
          ? (step > 0 ? ">" : "<")
          : (isTurnPoint(points, index + 1, x, current.y) ? "+" : "-")
        cells.set(`${x}:${current.y}`, { x, y: current.y, char })
        if (atEnd) {
          break
        }
      }
    }
  }
  return [...cells.values()]
    .sort((left, right) => left.y - right.y || left.x - right.x)
}

function isTurnPoint(
  points: Array<{ x: number; y: number }>,
  pointIndex: number,
  x: number,
  y: number,
) {
  const turn = points[pointIndex]
  if (!turn || turn.x !== x || turn.y !== y) {
    return false
  }
  return pointIndex > 0 && pointIndex < points.length - 1
}
