import type {
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
  WaitingRoomTerminal,
  WaitingRoomTerminalType,
} from "./waiting-room-types.js"

export function waitingRoomTerminalRows(
  state: Pick<WaitingRoomState, "focus" | "terminalIndex">,
  remote: Pick<WaitingRoomRemoteState, "terminals">,
  titleWidth: number,
): WaitingRoomRow[] {
  const terminals = waitingRoomTerminals(remote)
  const typeWidth = Math.max(
    "Type".length,
    ...terminals.map((terminal) => formatWaitingRoomTerminalType(terminal.terminal_type).length),
  )
  const rows: WaitingRoomRow[] = [
    {
      id: "terminals-header",
      title: "Terminals",
      value: "",
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
    {
      id: "terminal-columns",
      title: "Terminal ID",
      value: "",
      titleWidth,
      columns: [formatWaitingRoomColumnHeader("Type", typeWidth)],
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
  ]

  for (const [index, terminal] of terminals.entries()) {
    rows.push({
      id: `terminal:${terminal.terminal_id}`,
      title: formatWaitingRoomTerminalTitle(terminal),
      value: formatWaitingRoomTerminalType(terminal.terminal_type),
      titleWidth,
      columns: [formatWaitingRoomColumn(formatWaitingRoomTerminalType(terminal.terminal_type), typeWidth)],
      indent: 1,
      focused: state.focus === "terminal" && state.terminalIndex === index,
      selectable: true,
      scrollbar: "",
    })
  }

  rows.push({
    id: "add-terminal",
    title: "Add New Terminal",
    value: "Press Enter",
    titleWidth,
    indent: 1,
    focused: state.focus === "add-terminal",
    selectable: true,
    scrollbar: "",
  })

  return rows
}

export function waitingRoomTerminals(remote: Pick<WaitingRoomRemoteState, "terminals">) {
  return remote.terminals ?? []
}

export function formatWaitingRoomTerminalTitle(terminal: WaitingRoomTerminal) {
  const label = terminal.alias ? `${terminal.terminal_id} (${terminal.alias})` : terminal.terminal_id
  return terminal.revoked ? `${label} (revoked)` : label
}

export function formatWaitingRoomTerminalType(value: WaitingRoomTerminalType) {
  switch (value) {
    case "web":
      return "Web terminal"
    case "ios":
      return "iOS terminal"
    case "android":
      return "Android terminal"
    case "cli":
    default:
      return "CLI"
  }
}

function formatWaitingRoomColumnHeader(label: string, width: number) {
  return label.padEnd(width, " ")
}

function formatWaitingRoomColumn(value: string, width: number) {
  return value.padEnd(width, " ")
}
