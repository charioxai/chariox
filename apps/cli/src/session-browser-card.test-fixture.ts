import type { SessionListEntry } from "./sessions.js"

export const sessionBrowserFixture: SessionListEntry = {
  id: "18ca9569919075f8",
  project_id: "project-1",
  alias: "workspace-2",
  worktree_id: "/workspace",
  status: "Parked",
  created_at_ms: 1_786_404_419_447,
  last_used_at_ms: 1_786_404_762_837,
  activity: {
    agent_count: 3,
    working_agent_count: 0,
    active_prompt_count: 0,
    queued_prompt_count: 0,
    unread_idle_agent_count: 2,
    error_agent_count: 1,
  },
  workflows: [{ id: "workflow-1", activity: { working: false } }],
  joined_collaborator_count: 1,
  pending_collaboration_invite_count: 1,
}
