import type {
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
  WaitingRoomTerminal,
  WaitingRoomTerminalType,
} from "./waiting-room-types.js"
import {
  activeProjectEnvironmentSetupStatus,
  safeProjectEnvironmentSetupFailureDetails,
  safeProjectEnvironmentSetupOperationId,
} from "./project-environment-setup-projection.js"
import type { ProjectEnvironmentSetupStatus } from "@chariox/kernel-client/kernel-types"

export function waitingRoomTerminalRows(
  state: Pick<WaitingRoomState, "focus" | "terminalIndex">,
  remote: Pick<WaitingRoomRemoteState, "terminals">,
  titleWidth: number,
  projectEnvironmentSetup: ProjectEnvironmentSetupStatus | null = activeProjectEnvironmentSetupStatus(),
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

  rows.push(...waitingRoomProjectEnvironmentSetupRows(projectEnvironmentSetup, titleWidth))

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

export function waitingRoomProjectEnvironmentSetupRows(
  status: ProjectEnvironmentSetupStatus | null | undefined,
  titleWidth: number,
): WaitingRoomRow[] {
  if (!status) {
    return []
  }
  const operationId = safeProjectEnvironmentSetupOperationId(status.operation_id)
  const progress = `${status.progress_percent}%`
  const failure = safeProjectEnvironmentSetupFailureDetails(status)
  const rows: WaitingRoomRow[] = [
    {
      id: "project-environment-setup-header",
      title: "Project environment",
      value: status.phase,
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
    {
      id: `project-environment-setup:${operationId}`,
      title: "Setup",
      value: `${operationId} · ${progress}`,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
  ]
  if (failure) {
    rows.push({
      id: "project-environment-setup-failure",
      title: "Failure",
      value: failure,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
  }
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
