import type { SessionListEntry } from "./sessions.js"
import {
  sessionBrowserStatus,
  sessionBrowserTimestamp,
  sessionBrowserTitle,
} from "./sessions.js"

export function sessionBrowserCardLines(session: SessionListEntry, selected: boolean) {
  const agentCount = session.activity?.agent_count ?? session.agents?.length ?? 0
  const working = session.activity?.working_agent_count ?? 0
  const done = session.activity?.unread_idle_agent_count ?? 0
  const error = session.activity?.error_agent_count ?? 0
  const idle = Math.max(0, agentCount - working - done - error)
  const workflows = session.workflows?.length ?? 0
  const running = session.workflows?.filter((workflow) => workflow.activity?.working).length ?? 0
  const joined = session.joined_collaborator_count ?? 0
  const pending = session.pending_collaboration_invite_count ?? 0
  return {
    title: `${selected ? ">" : " "} ${sessionBrowserTitle(session)} · ${sessionBrowserStatus(session)}`,
    timestamps: `  last ${sessionBrowserTimestamp(session.last_used_at_ms ?? null)} · created ${sessionBrowserTimestamp(session.created_at_ms ?? null)}`,
    agents: `  ${[
      `${agentCount} agents`,
      ...(working ? [`${working} WORKING`] : []),
      ...(idle ? [`${idle} IDLE`] : []),
      ...(done ? [`${done} DONE`] : []),
      ...(error ? [`${error} ERROR`] : []),
    ].join(" · ")}`,
    workflows: `  ${[
      `${workflows} workflows`,
      ...(running ? [`${running} RUNNING`] : []),
    ].join(" · ")}`,
    collaborations: `  ${joined} collaborators joined · ${pending} invitations pending`,
  }
}
