import type { SessionListEntry } from "./sessions.js"
import { waitingRoomRemoteKernels, waitingRoomRemoteMachines } from "./waiting-room-remote-rows.js"
import { waitingRoomPreviewSessions, waitingRoomSessions } from "./waiting-room-session-rows.js"
import { waitingRoomSlices } from "./waiting-room-slices.js"
import { waitingRoomTerminals } from "./waiting-room-terminal-rows.js"
import type { WaitingRoomFocus, WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

export type WaitingRoomFocusTarget = {
  focus: WaitingRoomFocus
  sessionIndex: number
  machineIndex: number
  remoteKernelIndex: number
  terminalIndex: number
}

export function moveWaitingRoomFocus(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  delta: number,
  remote: WaitingRoomRemoteState = {},
) {
  const order = waitingRoomFocusTargets(sessions, remote)
  const currentIndex = Math.max(
    0,
    order.findIndex((target) => (
      target.focus === state.focus
      && (target.focus !== "session" || target.sessionIndex === state.sessionIndex)
      && (target.focus !== "machine" || target.machineIndex === state.machineIndex)
      && (target.focus !== "remote-kernel" || target.remoteKernelIndex === state.remoteKernelIndex)
      && (target.focus !== "terminal" || target.terminalIndex === state.terminalIndex)
    )),
  )
  const next = order[modulo(currentIndex + delta, order.length)] ?? order[0]
  if (!next) {
    return state
  }

  return {
    ...state,
    focus: next.focus,
    sessionIndex: next.focus === "session" ? next.sessionIndex : state.sessionIndex,
    machineIndex: next.focus === "machine" ? next.machineIndex : state.machineIndex,
    remoteKernelIndex: next.focus === "remote-kernel" ? next.remoteKernelIndex : state.remoteKernelIndex,
    terminalIndex: next.focus === "terminal" ? next.terminalIndex : state.terminalIndex,
  }
}

export function waitingRoomFocusTargets(
  sessions: SessionListEntry[],
  remote: WaitingRoomRemoteState = {},
): WaitingRoomFocusTarget[] {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const terminals = waitingRoomTerminals(remote)
  const slices = waitingRoomSlices(remote)
  return [
    { focus: "new" as const, sessionIndex: 0 },
    { focus: "provider" as const, sessionIndex: 0 },
    { focus: "model" as const, sessionIndex: 0 },
    { focus: "effort" as const, sessionIndex: 0 },
    { focus: "workspace" as const, sessionIndex: 0 },
    { focus: "worktree" as const, sessionIndex: 0 },
    ...(slices.length > 0 ? [{ focus: "slice" as const, sessionIndex: 0 }] : []),
    ...(visibleSessions.length > 0 ? [{ focus: "join-sessions" as const, sessionIndex: 0 }] : []),
    ...previewSessions.map((session) => ({
      focus: "session" as const,
      sessionIndex: Math.max(0, visibleSessions.findIndex((candidate) => candidate.id === session.id)),
    })),
    { focus: "relay" as const, sessionIndex: 0 },
    ...remoteMachines.map((_, machineIndex) => ({
      focus: "machine" as const,
      sessionIndex: 0,
      machineIndex,
    })),
    ...remoteKernels.map((_, remoteKernelIndex) => ({
      focus: "remote-kernel" as const,
      sessionIndex: 0,
      remoteKernelIndex,
    })),
    ...terminals.map((_, terminalIndex) => ({
      focus: "terminal" as const,
      sessionIndex: 0,
      terminalIndex,
    })),
    { focus: "add-terminal" as const, sessionIndex: 0 },
    { focus: "theme" as const, sessionIndex: 0 },
  ].map((target) => ({
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    ...target,
  }))
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}
