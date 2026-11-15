import type { ContextDevelopmentSetup } from "@chariox/kernel-client/context-transfer-types"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { projectSelectionFromId } from "./waiting-room-projects.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"

export function normalizeWaitingRoomManagedDraft(state: WaitingRoomState, remote: WaitingRoomRemoteState): WaitingRoomState {
  const options = waitingRoomProjectRepositoryOptions(state, remote)
  const project = selectedManagedProject(state, remote)
  const primary = options[0]?.workspaceId
  const supporting = options.slice(1).map(option => option.workspaceId)
  const existing = state.managedRepositorySelection
  const normalized: WaitingRoomState = {
    ...state, managedDevelopmentMode: state.managedDevelopmentMode ?? "empty",
    managedRepositoryIndex: supporting.length ? modulo(state.managedRepositoryIndex ?? 0, supporting.length) : 0,
  }
  if (project && primary) normalized.managedRepositorySelection = {
    projectId: project.id, primaryWorkspaceId: primary,
    supportingWorkspaceIds: existing?.projectId === project.id && existing.primaryWorkspaceId === primary
      ? supporting.filter(id => existing.supportingWorkspaceIds.includes(id)) : supporting,
  }
  else delete normalized.managedRepositorySelection
  return normalized
}

export function cycleManagedDevelopment(state: WaitingRoomState): WaitingRoomState {
  return {
    ...state,
    managedDevelopmentMode: state.managedDevelopmentMode === "empty" ? "current_project" : "empty",
  }
}

export function toggleManagedRepositorySelection(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): WaitingRoomState {
  const normalized = normalizeWaitingRoomManagedDraft(state, remote)
  const selection = normalized.managedRepositorySelection
  const option = waitingRoomProjectRepositoryOptions(normalized, remote)
    .slice(1)[normalized.managedRepositoryIndex ?? 0]
  if (!selection || !option) return normalized
  const selected = new Set(selection.supportingWorkspaceIds)
  if (selected.has(option.workspaceId)) selected.delete(option.workspaceId)
  else selected.add(option.workspaceId)
  const supportingWorkspaceIds = waitingRoomProjectRepositoryOptions(normalized, remote)
    .slice(1)
    .map((candidate) => candidate.workspaceId)
    .filter((workspaceId) => selected.has(workspaceId))
  return {
    ...normalized,
    managedRepositorySelection: { ...selection, supportingWorkspaceIds },
  }
}

export function waitingRoomProjectDevelopmentSetup(
  state: Pick<WaitingRoomState,
    | "projectSelectionId"
    | "managedDevelopmentMode"
    | "managedRepositorySelection"
  >,
  remote: WaitingRoomRemoteState,
): ContextDevelopmentSetup | null {
  if (state.managedDevelopmentMode === "empty") return { kind: "empty" }
  if (state.managedDevelopmentMode !== "current_project") return null
  if (!remote.workspaceId) return null
  const project = selectedManagedProject(state, remote)
  if (!project) return null
  const options = waitingRoomProjectRepositoryOptions(state, remote)
  const primaryWorkspaceId = options[0]?.workspaceId ?? ""
  const selection = state.managedRepositorySelection
  const selectedSupporting = selection
    && selection.projectId === project.id
    && selection.primaryWorkspaceId === primaryWorkspaceId
      ? new Set(selection.supportingWorkspaceIds)
      : new Set(options.slice(1).map((option) => option.workspaceId))
  const repositories = options
    .filter((option) => option.primary || selectedSupporting.has(option.workspaceId))
    .map((option) => ({
      role: option.primary ? "primary" as const : "supporting" as const,
      workspaceId: option.workspaceId,
      worktreeId: option.primary ? remote.worktreeId ?? null : null,
    }))
  return { kind: "source_project", projectId: project.id, repositories }
}

export function waitingRoomProjectRepositoryOptions(
  state: Pick<WaitingRoomState, "projectSelectionId">,
  remote: WaitingRoomRemoteState,
): Array<{ workspaceId: string; primary: boolean }> {
  const project = selectedManagedProject(state, remote)
  if (!project) return []
  const primaryWorkspaceId = remote.workspaceId ?? ""
  const workspaceIds = project.workspace_ids?.length ? project.workspace_ids : [project.workspace_id]
  return [primaryWorkspaceId, ...workspaceIds.filter((workspaceId) => workspaceId !== primaryWorkspaceId)]
    .filter((workspaceId, index, values) => workspaceId && values.indexOf(workspaceId) === index)
    .map((workspaceId, index) => ({ workspaceId, primary: index === 0 }))
}

function selectedManagedProject(
  state: Pick<WaitingRoomState, "projectSelectionId">,
  remote: WaitingRoomRemoteState,
): WaitingRoomProjectSummary | null {
  const selection = projectSelectionFromId(state.projectSelectionId ?? "default")
  if (selection.kind === "existing") {
    return remote.projects?.find((project) => project.id === selection.project_id) ?? null
  }
  if (selection.kind === "default") {
    return remote.projects?.find((project) => (
      project.kind === "default"
      && (project.workspace_ids?.includes(remote.workspaceId ?? "")
        || project.workspace_id === remote.workspaceId)
    )) ?? null
  }
  return null
}

function modulo(value: number, size: number): number {
  if (size <= 0) return 0
  return ((value % size) + size) % size
}
