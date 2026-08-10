import type { SessionListEntry } from "./sessions.js"
import {
  activeWaitingRoomProjects,
  archivedWaitingRoomProjects,
  sessionsForProject,
  type WaitingRoomProjectSummary,
} from "./waiting-room-projects.js"
import type { WaitingRoomRow, WaitingRoomState } from "./waiting-room-types.js"

export function waitingRoomProjectRows(
  state: Pick<WaitingRoomState, "focus" | "projectIndex" | "showArchivedProjects">,
  projects: readonly WaitingRoomProjectSummary[] | undefined,
  sessions: readonly SessionListEntry[],
  options: { inventoryLoading: boolean; loadingText: string; titleWidth: number },
): WaitingRoomRow[] {
  const active = activeWaitingRoomProjects(projects)
  const archived = archivedWaitingRoomProjects(projects)
  if (active.length === 0 && archived.length === 0) {
    return [{
      id: options.inventoryLoading ? "projects-loading" : "no-projects",
      title: "Projects",
      value: options.inventoryLoading ? options.loadingText : "No projects available",
      titleWidth: options.titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    }]
  }
  const all = [...active, ...(state.showArchivedProjects ? archived : [])]
  const rows: WaitingRoomRow[] = [{
    id: "project-header",
    title: "Projects",
    value: "Enter browses sessions • E renames • A/D confirm • R restores archived",
    titleWidth: options.titleWidth,
    indent: 0,
    focused: false,
    selectable: false,
    scrollbar: "",
  }]
  if (archived.length > 0) {
    rows.push({
      id: "archived-projects",
      title: "Archived projects",
      value: `${state.showArchivedProjects ? "shown" : "hidden"} · ${archived.length} project${archived.length === 1 ? "" : "s"} · use left/right`,
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "archived-projects",
      selectable: true,
      scrollbar: "",
    })
  }
  for (const [projectIndex, project] of all.entries()) {
    const projectSessions = sessionsForProject(sessions, project.id)
    rows.push({
      id: `project-entry:${project.id}`,
      title: project.name,
      value: formatWaitingRoomProjectSummary(project, projectSessions),
      titleWidth: options.titleWidth,
      indent: 1,
      focused: state.focus === "project-entry" && (state.projectIndex ?? 0) === projectIndex,
      selectable: true,
      scrollbar: "",
    })
  }
  return rows
}

export function waitingRoomProjectsForNavigation(
  projects: readonly WaitingRoomProjectSummary[] | undefined,
  includeArchived = true,
): WaitingRoomProjectSummary[] {
  return [
    ...activeWaitingRoomProjects(projects),
    ...(includeArchived ? archivedWaitingRoomProjects(projects) : []),
  ]
}

export function formatWaitingRoomProjectSummary(
  project: WaitingRoomProjectSummary,
  sessions: readonly SessionListEntry[],
): string {
  const activity = aggregateProjectSessionActivity(sessions)
  const sessionCount = project.session_count ?? sessions.length
  const parts = [
    `${sessionCount} session${sessionCount === 1 ? "" : "s"}`,
    `${activity.agentCount} agents`,
  ]
  if (activity.working > 0) parts.push(`${activity.working} WORKING`)
  if (activity.idle > 0) parts.push(`${activity.idle} IDLE`)
  if (activity.done > 0) parts.push(`${activity.done} DONE`)
  if (activity.error > 0) parts.push(`${activity.error} ERROR`)
  parts.push(`${activity.workflowCount} workflows`)
  if (activity.runningWorkflows > 0) parts.push(`${activity.runningWorkflows} RUNNING`)
  parts.push(`${project.joined_collaborator_count} collaborators joined`)
  parts.push(`${project.pending_collaboration_invite_count} invitations pending`)
  if (project.status === "archived") parts.push("ARCHIVED")
  return parts.join(" · ")
}

function aggregateProjectSessionActivity(sessions: readonly SessionListEntry[]) {
  let agentCount = 0
  let working = 0
  let done = 0
  let error = 0
  let workflowCount = 0
  let runningWorkflows = 0
  for (const session of sessions) {
    agentCount += session.activity?.agent_count ?? session.agents?.length ?? 0
    working += session.activity?.working_agent_count ?? 0
    done += session.activity?.unread_idle_agent_count ?? 0
    error += session.activity?.error_agent_count ?? 0
    workflowCount += session.workflows?.length ?? 0
    runningWorkflows += session.workflows?.filter((workflow) => workflow.activity?.working).length ?? 0
  }
  return {
    agentCount,
    working,
    done,
    error,
    idle: Math.max(0, agentCount - working - done - error),
    workflowCount,
    runningWorkflows,
  }
}
