export type PaneGridTone = "subtle" | "focused"

export type PaneGridAgent = {
  id: string
}

export type PaneGridSlot = {
  paneIndex: number
  agentId: string | null
  visible: boolean
  focused: boolean
  colStart: 0 | 1
  colSpan: 1 | 2
}

export type PaneGridVerticalSegment = {
  visible: boolean
  tone: PaneGridTone
}

export type PaneGridHorizontalSegment = {
  visible: boolean
  tone: PaneGridTone
}

export type PaneGridJunction = {
  visible: boolean
  char: string
  tone: PaneGridTone
}

export type PaneGridContentRow = {
  rowIndex: number
  visible: boolean
  slots: PaneGridSlot[]
  verticals: [PaneGridVerticalSegment, PaneGridVerticalSegment, PaneGridVerticalSegment]
}

export type PaneGridBorderRow = {
  rowIndex: number
  visible: boolean
  horizontals: [PaneGridHorizontalSegment, PaneGridHorizontalSegment]
  junctions: [PaneGridJunction, PaneGridJunction, PaneGridJunction]
}

export type PaneGridModel = {
  split: boolean
  rows: PaneGridContentRow[]
  borderRows: PaneGridBorderRow[]
}

export function buildPaneGridModel<TAgent extends PaneGridAgent>(options: {
  paneRows: readonly (readonly number[])[]
  visibleAgents: readonly TAgent[]
  focusedAgentId: string | null
  split: boolean
  showWorkflowScreen: boolean
}): PaneGridModel {
  const rows = options.paneRows.map((rowSlots, rowIndex): PaneGridContentRow => {
    const visiblePaneIndices = rowSlots.filter((paneIndex) => paneVisible({
      paneIndex,
      visibleAgents: options.visibleAgents,
      split: options.split,
      showWorkflowScreen: options.showWorkflowScreen,
    }))
    const fullSpan = visiblePaneIndices.length === 1
    const slots = visiblePaneIndices.map((paneIndex, position): PaneGridSlot => {
      const agent = options.visibleAgents[paneIndex] ?? null
      return {
        paneIndex,
        agentId: agent?.id ?? null,
        visible: true,
        focused: Boolean(agent?.id && agent.id === options.focusedAgentId),
        colStart: fullSpan ? 0 : position === 0 ? 0 : 1,
        colSpan: fullSpan ? 2 : 1,
      }
    })
    const visible = slots.length > 0
    return {
      rowIndex,
      visible,
      slots,
      verticals: [
        verticalSegment(options.split, slots, 0),
        verticalSegment(options.split, slots, 1),
        verticalSegment(options.split, slots, 2),
      ],
    }
  })

  return {
    split: options.split,
    rows,
    borderRows: Array.from({ length: rows.length + 1 }, (_, rowIndex) => borderRow(options.split, rows, rowIndex)),
  }
}

function paneVisible<TAgent extends PaneGridAgent>(options: {
  paneIndex: number
  visibleAgents: readonly TAgent[]
  split: boolean
  showWorkflowScreen: boolean
}) {
  if (options.paneIndex === 0) {
    return true
  }
  return !options.showWorkflowScreen && options.split && Boolean(options.visibleAgents[options.paneIndex])
}

function verticalSegment(
  split: boolean,
  slots: PaneGridSlot[],
  boundary: 0 | 1 | 2,
): PaneGridVerticalSegment {
  const visible = slots.some((slot) => slot.visible && (leftBoundary(slot) === boundary || rightBoundary(slot) === boundary))
  const focused = slots.some((slot) => slot.focused && (leftBoundary(slot) === boundary || rightBoundary(slot) === boundary))
  return {
    visible: split ? visible : boundary === 0 && slots.length > 0,
    tone: focused ? "focused" : "subtle",
  }
}

function borderRow(split: boolean, rows: PaneGridContentRow[], rowIndex: number): PaneGridBorderRow {
  const rowAbove = rows[rowIndex - 1] ?? null
  const rowBelow = rows[rowIndex] ?? null
  const visible = split && Boolean(rowAbove?.visible || rowBelow?.visible)
  const horizontals: [PaneGridHorizontalSegment, PaneGridHorizontalSegment] = [
    horizontalSegment(visible, rowAbove, rowBelow, 0),
    horizontalSegment(visible, rowAbove, rowBelow, 1),
  ]
  const junctions: [PaneGridJunction, PaneGridJunction, PaneGridJunction] = [0, 1, 2].map((boundary) => {
    const left = visible && boundary > 0
    const right = visible && boundary < 2
    const up = Boolean(rowAbove?.verticals[boundary]?.visible)
    const down = Boolean(rowBelow?.verticals[boundary]?.visible)
    const focused = Boolean(
      (boundary > 0 && horizontals[boundary - 1]?.tone === "focused")
      || (boundary < 2 && horizontals[boundary]?.tone === "focused")
      || rowAbove?.verticals[boundary]?.tone === "focused"
      || rowBelow?.verticals[boundary]?.tone === "focused",
    )
    return {
      visible,
      char: visible ? junctionChar({ left, right, up, down }) : " ",
      tone: focused ? "focused" : "subtle",
    }
  }) as [PaneGridJunction, PaneGridJunction, PaneGridJunction]

  return {
    rowIndex,
    visible,
    horizontals,
    junctions,
  }
}

function horizontalSegment(
  visible: boolean,
  rowAbove: PaneGridContentRow | null,
  rowBelow: PaneGridContentRow | null,
  col: 0 | 1,
): PaneGridHorizontalSegment {
  const focused = Boolean(
    rowAbove?.slots.some((slot) => slot.focused && slotContainsCol(slot, col))
    || rowBelow?.slots.some((slot) => slot.focused && slotContainsCol(slot, col)),
  )
  return {
    visible,
    tone: focused ? "focused" : "subtle",
  }
}

function slotContainsCol(slot: PaneGridSlot, col: 0 | 1) {
  return slot.colStart <= col && col < slot.colStart + slot.colSpan
}

function leftBoundary(slot: PaneGridSlot): 0 | 1 | 2 {
  return slot.colStart
}

function rightBoundary(slot: PaneGridSlot): 0 | 1 | 2 {
  return (slot.colStart + slot.colSpan) as 0 | 1 | 2
}

function junctionChar(edges: { left: boolean; right: boolean; up: boolean; down: boolean }) {
  const { left, right, up, down } = edges
  if (left && right && up && down) {
    return "┼"
  }
  if (left && right && down) {
    return "┬"
  }
  if (left && right && up) {
    return "┴"
  }
  if (up && down && right) {
    return "├"
  }
  if (up && down && left) {
    return "┤"
  }
  if (down && right) {
    return "┌"
  }
  if (down && left) {
    return "┐"
  }
  if (up && right) {
    return "└"
  }
  if (up && left) {
    return "┘"
  }
  if (left || right) {
    return "─"
  }
  if (up || down) {
    return "│"
  }
  return " "
}
