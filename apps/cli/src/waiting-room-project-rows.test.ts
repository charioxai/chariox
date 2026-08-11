import assert from "node:assert/strict"
import test from "node:test"

import { waitingRoomProjectRows } from "./waiting-room-project-rows.js"
import type { SessionListEntry } from "./sessions.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"

test("project rows expose nonzero runtime badges and separate collaboration counts", () => {
  const project = projectSummary()
  const session = {
    id: "session-1",
    project_id: project.id,
    worktree_id: "/repo",
    status: "Active",
    activity: {
      agent_count: 7,
      working_agent_count: 2,
      active_prompt_count: 2,
      queued_prompt_count: 0,
      error_agent_count: 1,
      unread_idle_agent_count: 2,
    },
    workflows: [
      { id: "running", activity: { working: true } },
      { id: "idle", activity: { working: false } },
    ],
  } as SessionListEntry & { workflows: Array<{ id: string; activity: { working: boolean } }> }

  const rows = waitingRoomProjectRows(
    { focus: "project-entry", projectIndex: 0 },
    [project],
    [session],
    { inventoryLoading: false, loadingText: "loading", titleWidth: 24 },
  )
  const row = rows.find((candidate) => candidate.id === `project-entry:${project.id}`)

  assert.equal(row?.focused, true)
  assert.equal(
    row?.value,
    "1 session · 7 agents · 2 WORKING · 2 IDLE · 2 DONE · 1 ERROR · 2 workflows · 1 RUNNING · 3 collaborators joined · 2 invitations pending",
  )
})

test("project rows hide archived projects until the archive filter is enabled", () => {
  const active = projectSummary()
  const archived = {
    ...projectSummary(),
    id: "project-archived",
    name: "Archived",
    status: "archived" as const,
    archived_at_ms: 5,
  }
  const hiddenRows = waitingRoomProjectRows(
    { focus: "archived-projects", projectIndex: 0, showArchivedProjects: false },
    [active, archived],
    [],
    { inventoryLoading: false, loadingText: "loading", titleWidth: 24 },
  )
  const visibleRows = waitingRoomProjectRows(
    { focus: "project-entry", projectIndex: 1, showArchivedProjects: true },
    [active, archived],
    [],
    { inventoryLoading: false, loadingText: "loading", titleWidth: 24 },
  )

  assert.equal(hiddenRows.some((row) => row.id === "project-entry:project-archived"), false)
  assert.match(hiddenRows.find((row) => row.id === "archived-projects")?.value ?? "", /hidden/)
  assert.equal(visibleRows.find((row) => row.id === "project-entry:project-archived")?.focused, true)
  assert.match(visibleRows.find((row) => row.id === "project-entry:project-archived")?.value ?? "", /ARCHIVED/)
})

function projectSummary(): WaitingRoomProjectSummary {
  return {
    id: "project-1",
    owner_user_id: "owner",
    workspace_id: "/repo",
    name: "Frontend",
    kind: "named",
    status: "active",
    created_at_ms: 1,
    updated_at_ms: 2,
    session_count: 1,
    last_session_activity_at_ms: 3,
    joined_collaborator_count: 3,
    pending_collaboration_invite_count: 2,
  }
}
