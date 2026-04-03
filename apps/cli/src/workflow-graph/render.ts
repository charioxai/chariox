export function buildWorkflowEdgeCells(points: Array<{ x: number; y: number }>) {
  const cells: Array<{ x: number; y: number; char: string }> = []
  if (points.length < 2) {
    return cells
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
          : "|"
        cells.push({ x: current.x, y, char })
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
          : "-"
        cells.push({ x, y: current.y, char })
        if (atEnd) {
          break
        }
      }
    }
  }
  return cells
}
