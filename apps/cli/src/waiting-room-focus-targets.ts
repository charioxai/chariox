import {
  externalProviderSessionPageHasMore,
  externalProviderSessionPageSessions,
  externalProviderSessionSelectionIndex,
} from "@chariox/kernel-client/external-provider-sessions"
import type { SessionListEntry } from "./sessions.js"
import { waitingRoomRemoteKernels, waitingRoomRemoteMachines } from "./waiting-room-remote-rows.js"
import { waitingRoomPreviewSessions, waitingRoomSessions } from "./waiting-room-session-rows.js"
import { waitingRoomAllSlices } from "./waiting-room-slice-rows.js"
import { waitingRoomTerminals } from "./waiting-room-terminal-rows.js"
import type { WaitingRoomFocus, WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
import { waitingRoomProjectsForNavigation } from "./waiting-room-project-rows.js"
import {
  waitingRoomConfiguresNewManagedMachine,
  waitingRoomProjectRepositoryOptions,
} from "./waiting-room-managed-environments.js"

export type WaitingRoomFocusTarget = {
  focus: WaitingRoomFocus
  sessionIndex: number
  projectIndex: number
  machineIndex: number
  remoteKernelIndex: number
  sliceIndex: number
  terminalIndex: number
  externalSessionIndex: number
  managedRepositoryIndex: number
}

export function moveWaitingRoomFocus(
  state: WaitingRoomState,
  sessions: SessionListEntry[],
  delta: number,
  remote: WaitingRoomRemoteState = {},
) {
  const order = waitingRoomFocusTargets(sessions, remote, state)
  const externalSessions = externalProviderSessionPageSessions(remote)
  const externalSessionIndex = externalProviderSessionSelectionIndex(externalSessions, {
    selectedExternalProviderSessionIndex: state.externalSessionIndex ?? null,
  })
  const currentIndex = Math.max(
    0,
    order.findIndex((target) => (
      target.focus === state.focus
      && (target.focus !== "managed-repositories"
        || target.managedRepositoryIndex === (state.managedRepositoryIndex ?? 0))
      && (target.focus !== "session" || target.sessionIndex === state.sessionIndex)
      && (target.focus !== "project-entry" || target.projectIndex === (state.projectIndex ?? 0))
      && (target.focus !== "machine" || target.machineIndex === state.machineIndex)
      && (target.focus !== "remote-kernel" || target.remoteKernelIndex === state.remoteKernelIndex)
      && (target.focus !== "slice-entry" || target.sliceIndex === (state.sliceIndex ?? 0))
      && (target.focus !== "terminal" || target.terminalIndex === state.terminalIndex)
      && (target.focus !== "external-session" || target.externalSessionIndex === externalSessionIndex)
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
    ...(next.focus === "project-entry"
      ? { projectIndex: next.projectIndex }
      : state.projectIndex !== undefined
        ? { projectIndex: state.projectIndex }
        : {}),
    machineIndex: next.focus === "machine" ? next.machineIndex : state.machineIndex,
    remoteKernelIndex: next.focus === "remote-kernel" ? next.remoteKernelIndex : state.remoteKernelIndex,
    terminalIndex: next.focus === "terminal" ? next.terminalIndex : state.terminalIndex,
    ...(next.focus === "managed-repositories"
      ? { managedRepositoryIndex: next.managedRepositoryIndex }
      : state.managedRepositoryIndex !== undefined
        ? { managedRepositoryIndex: state.managedRepositoryIndex }
        : {}),
    ...(next.focus === "external-session"
      ? { externalSessionIndex: next.externalSessionIndex }
      : state.focus === "external-session"
        ? { externalSessionIndex }
        : {}),
  }
  return next.focus === "slice-entry"
    ? { ...nextState, sliceIndex: next.sliceIndex }
    : nextState
}

export function waitingRoomFocusTargets(
  sessions: SessionListEntry[],
  remote: WaitingRoomRemoteState = {},
  state?: Pick<WaitingRoomState,
    | "showArchivedProjects"
    | "selectedMachineRef"
    | "managedAutoStopPreset"
    | "managedDevelopmentMode"
    | "projectSelectionId"
    | "sliceSelectionId"
  >,
): WaitingRoomFocusTarget[] {
  const visibleSessions = waitingRoomSessions(sessions)
  const previewSessions = waitingRoomPreviewSessions(sessions)
  const remoteMachines = waitingRoomRemoteMachines(remote)
  const remoteKernels = waitingRoomRemoteKernels(remote)
  const slices = waitingRoomAllSlices(remote)
  const terminals = waitingRoomTerminals(remote)
  const externalSessions = externalProviderSessionPageSessions(remote)
  const projects = waitingRoomProjectsForNavigation(remote.projects, Boolean(state?.showArchivedProjects))
  const archivedProjectCount = (remote.projects ?? []).filter((project) => project.status === "archived").length
  const managedConfiguration = waitingRoomConfiguresNewManagedMachine(state?.selectedMachineRef)
  const sliceDevelopmentConfiguration = !managedConfiguration
    && Boolean(state?.sliceSelectionId && state.sliceSelectionId !== "none")
  const managedRepositoryOptions = state?.managedDevelopmentMode === "current_project"
    ? waitingRoomProjectRepositoryOptions(state, remote).slice(1)
    : []
  return [
    { focus: "new" as const, sessionIndex: 0 },
    { focus: "launch-machine" as const, sessionIndex: 0 },
    { focus: "launch-kernel" as const, sessionIndex: 0 },
    ...(managedConfiguration
      ? [
          { focus: "managed-compute" as const, sessionIndex: 0 },
          { focus: "managed-region" as const, sessionIndex: 0 },
          { focus: "managed-kernel-context" as const, sessionIndex: 0 },
          { focus: "managed-development" as const, sessionIndex: 0 },
          ...managedRepositoryOptions.map((_, managedRepositoryIndex) => ({
            focus: "managed-repositories" as const,
            sessionIndex: 0,
            managedRepositoryIndex,
          })),
          { focus: "managed-provider-accounts" as const, sessionIndex: 0 },
          { focus: "managed-git-credentials" as const, sessionIndex: 0 },
          { focus: "managed-auto-stop" as const, sessionIndex: 0 },
          ...(state?.managedAutoStopPreset === "custom"
            ? [
                { focus: "managed-custom-minimum" as const, sessionIndex: 0 },
                { focus: "managed-custom-idle" as const, sessionIndex: 0 },
              ]
            : []),
        ]
      : []),
    ...(remote.projects !== undefined ? [{ focus: "project" as const, sessionIndex: 0 }] : []),
    { focus: "provider" as const, sessionIndex: 0 },
    { focus: "account" as const, sessionIndex: 0 },
    { focus: "model" as const, sessionIndex: 0 },
    { focus: "effort" as const, sessionIndex: 0 },
    { focus: "workspace" as const, sessionIndex: 0 },
    { focus: "worktree" as const, sessionIndex: 0 },
    { focus: "live-sync" as const, sessionIndex: 0 },
    { focus: "collaborators" as const, sessionIndex: 0 },
    { focus: "slice" as const, sessionIndex: 0 },
    ...(sliceDevelopmentConfiguration
      ? [
          { focus: "managed-development" as const, sessionIndex: 0 },
          ...managedRepositoryOptions.map((_, managedRepositoryIndex) => ({
            focus: "managed-repositories" as const,
            sessionIndex: 0,
            managedRepositoryIndex,
          })),
        ]
      : []),
    ...(visibleSessions.length > 0 || projects.length > 0 ? [{ focus: "join-sessions" as const, sessionIndex: 0 }] : []),
    ...(archivedProjectCount > 0 ? [{ focus: "archived-projects" as const, sessionIndex: 0 }] : []),
    ...(projects.length > 0
      ? projects.map((_, projectIndex) => ({ focus: "project-entry" as const, sessionIndex: 0, projectIndex }))
      : previewSessions.map((session) => ({
          focus: "session" as const,
          sessionIndex: Math.max(0, visibleSessions.findIndex((candidate) => candidate.id === session.id)),
        }))),
    ...externalSessions.map((_, externalSessionIndex) => ({
      focus: "external-session" as const,
      sessionIndex: 0,
      externalSessionIndex,
    })),
    ...(externalProviderSessionPageHasMore(remote)
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
    { focus: "provider-accounts" as const, sessionIndex: 0 },
    { focus: "theme" as const, sessionIndex: 0 },
  ].map((target) => ({
    machineIndex: 0,
    remoteKernelIndex: 0,
    sliceIndex: 0,
    terminalIndex: 0,
    externalSessionIndex: 0,
    projectIndex: 0,
    managedRepositoryIndex: 0,
    ...target,
  }))
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}
