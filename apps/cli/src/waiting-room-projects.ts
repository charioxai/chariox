import type { SessionListEntry } from "./sessions.js"
import type {
  SessionProjectSelection,
  WaitingRoomPublicProjectSummary,
} from "@chariox/kernel-client"

export type WaitingRoomProjectSummary = WaitingRoomPublicProjectSummary
export type { SessionProjectSelection }

export const DEFAULT_PROJECT_SELECTION_ID = "default"
export const NEW_PROJECT_SELECTION_ID = "new"
const EXISTING_PROJECT_SELECTION_PREFIX = "existing:"

export function existingProjectSelectionId(projectId: string): string {
  return `${EXISTING_PROJECT_SELECTION_PREFIX}${projectId}`
}

export function projectSelectionFromId(selectionId: string): SessionProjectSelection {
  if (selectionId === NEW_PROJECT_SELECTION_ID) {
    return { kind: "new" }
  }
  if (selectionId.startsWith(EXISTING_PROJECT_SELECTION_PREFIX)) {
    const projectId = selectionId.slice(EXISTING_PROJECT_SELECTION_PREFIX.length).trim()
    if (projectId) {
      return { kind: "existing", project_id: projectId }
    }
  }
  return { kind: "default" }
}

export function waitingRoomProjectOptions(
  projects: readonly WaitingRoomProjectSummary[] | undefined,
  workspaceId?: string | null,
): Array<{ id: string; label: string; project?: WaitingRoomProjectSummary }> {
  const existing = (projects ?? [])
    .filter((project) => project.status === "active" && project.kind === "named")
    .filter((project) => !workspaceId || project.workspace_id === workspaceId)
    .sort((left, right) => (
      (right.last_session_activity_at_ms ?? right.updated_at_ms)
      - (left.last_session_activity_at_ms ?? left.updated_at_ms)
      || left.name.localeCompare(right.name)
    ))
  return [
    { id: DEFAULT_PROJECT_SELECTION_ID, label: "Default" },
    ...existing.map((project) => ({
      id: existingProjectSelectionId(project.id),
      label: project.name,
      project,
    })),
    { id: NEW_PROJECT_SELECTION_ID, label: "New" },
  ]
}

export function normalizeWaitingRoomProjectSelectionId(
  selectionId: string | null | undefined,
  projects: readonly WaitingRoomProjectSummary[] | undefined,
  workspaceId?: string | null,
): string {
  const options = waitingRoomProjectOptions(projects, workspaceId)
  return options.some((option) => option.id === selectionId)
    ? selectionId!
    : DEFAULT_PROJECT_SELECTION_ID
}

export function cycleWaitingRoomProjectSelectionId(
  selectionId: string,
  projects: readonly WaitingRoomProjectSummary[] | undefined,
  delta: number,
  workspaceId?: string | null,
): string {
  const options = waitingRoomProjectOptions(projects, workspaceId)
  const index = Math.max(0, options.findIndex((option) => option.id === selectionId))
  return options[modulo(index + delta, options.length)]?.id ?? DEFAULT_PROJECT_SELECTION_ID
}

export function describeWaitingRoomProjectSelection(
  selectionId: string,
  projects: readonly WaitingRoomProjectSummary[] | undefined,
  workspaceId?: string | null,
): string {
  return waitingRoomProjectOptions(projects, workspaceId)
    .find((option) => option.id === selectionId)?.label ?? "Default"
}

export function sessionsForProject(
  sessions: readonly SessionListEntry[],
  projectId: string,
): SessionListEntry[] {
  return sessions.filter((session) => session.project_id === projectId)
}

export function activeWaitingRoomProjects(
  projects: readonly WaitingRoomProjectSummary[] | undefined,
): WaitingRoomProjectSummary[] {
  return (projects ?? [])
    .filter((project) => project.status === "active")
    .sort((left, right) => (
      (right.last_session_activity_at_ms ?? right.updated_at_ms)
      - (left.last_session_activity_at_ms ?? left.updated_at_ms)
      || left.name.localeCompare(right.name)
    ))
}

export function archivedWaitingRoomProjects(
  projects: readonly WaitingRoomProjectSummary[] | undefined,
): WaitingRoomProjectSummary[] {
  return (projects ?? [])
    .filter((project) => project.status === "archived")
    .sort((left, right) => (right.archived_at_ms ?? right.updated_at_ms) - (left.archived_at_ms ?? left.updated_at_ms))
}

function modulo(value: number, size: number): number {
  if (size <= 0) return 0
  return ((value % size) + size) % size
}
