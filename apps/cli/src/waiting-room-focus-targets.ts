import type { SessionListEntry } from "./sessions.js"
import { waitingRoomRemoteKernels, waitingRoomRemoteMachines } from "./waiting-room-remote-rows.js"
import { waitingRoomPreviewSessions, waitingRoomSessions } from "./waiting-room-session-rows.js"
import { waitingRoomAllSlices } from "./waiting-room-slice-rows.js"
import { waitingRoomTerminals } from "./waiting-room-terminal-rows.js"
import type { WaitingRoomFocus, WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

export type WaitingRoomFocusTarget = {
  focus: WaitingRoomFocus
  sessionIndex: number
  machineIndex: number
  remoteKernelIndex: number
  sliceIndex: number
  terminalIndex: number
  externalSessionIndex: number
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
      && (target.focus !== "slice-entry" || target.sliceIndex === (state.sliceIndex ?? 0))
      && (target.focus !== "terminal" || target.terminalIndex === state.terminalIndex)
      && (target.focus !== "external-session" || target.externalSessionIndex === (state.externalSessionIndex ?? 0))
    )),
  )
  const next = order[modulo(currentIndex + delta, order.length)] ?? order[0]
  if (!next) {
    return state
  }

  const nextState: WaitingRoomState = {
    ...state,
    focus: next.focus,
    sessionIndex: next.focus === "session" ? next.sessionIndex : state.sessionIndex,
    machineIndex: next.focus === "machine" ? next.machineIndex : state.machineIndex,
    remoteKernelIndex: next.focus === "remote-kernel" ? next.remoteKernelIndex : state.remoteKernelIndex,
    terminalIndex: next.focus === "terminal" ? next.terminalIndex : state.terminalIndex,
    ...(next.focus === "external-session" ? { externalSessionIndex: next.externalSessionIndex } : {}),
  }
  return next.focus === "slice-entry"
    ? { ...nextState, sliceIndex: next.sliceIndex }
    : nextState
}

export function waitingRoomFocusTargets(
  sessions: SessionListEntry[],
  remote: WaitingRoomRemoteState = {},
): WaitingRoomFocusTarget[] {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const slices = waitingRoomAllSlices(remote)
  const terminals = waitingRoomTerminals(remote)
  const externalSessions = remote.externalProviderSessions ?? []
  return [
    { focus: "new" as const, sessionIndex: 0 },
    { focus: "launch-machine" as const, sessionIndex: 0 },
    { focus: "launch-kernel" as const, sessionIndex: 0 },
    { focus: "provider" as const, sessionIndex: 0 },
    { focus: "model" as const, sessionIndex: 0 },
    { focus: "effort" as const, sessionIndex: 0 },
    { focus: "workspace" as const, sessionIndex: 0 },
    { focus: "worktree" as const, sessionIndex: 0 },
    { focus: "live-sync" as const, sessionIndex: 0 },
    { focus: "collaborators" as const, sessionIndex: 0 },
    { focus: "slice" as const, sessionIndex: 0 },
    ...(visibleSessions.length > 0 ? [{ focus: "join-sessions" as const, sessionIndex: 0 }] : []),
    ...previewSessions.map((session) => ({
      focus: "session" as const,
      sessionIndex: Math.max(0, visibleSessions.findIndex((candidate) => candidate.id === session.id)),
    })),
    ...externalSessions.map((_, externalSessionIndex) => ({
      focus: "external-session" as const,
      sessionIndex: 0,
      externalSessionIndex,
    })),
    ...(remote.externalProviderSessionsHasMore
      ? [{ focus: "external-session-more" as const, sessionIndex: 0 }]
      : []),
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
    ...slices.map((_, sliceIndex) => ({
      focus: "slice-entry" as const,
      sessionIndex: 0,
      sliceIndex,
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
    sliceIndex: 0,
    terminalIndex: 0,
    externalSessionIndex: 0,
    ...target,
  }))
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}
