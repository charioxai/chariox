import assert from "node:assert/strict"
import test from "node:test"

import {
  cycleWaitingRoomProjectSelectionId,
  describeWaitingRoomProjectSelection,
  existingProjectSelectionId,
  projectSelectionFromId,
  waitingRoomProjectOptions,
  type WaitingRoomProjectSummary,
} from "./waiting-room-projects.js"

test("project selector offers Default, same-workspace named projects, then New", () => {
  const projects = [
    project("default", "arroba", "default", "/repo", 100),
    project("other", "Other", "named", "/other", 500),
    project("older", "Docs", "named", "/repo", 200),
    project("newer", "Frontend", "named", "/repo", 400),
    { ...project("archived", "Old", "named", "/repo", 800), status: "archived" as const },
  ]

  assert.deepEqual(
    waitingRoomProjectOptions(projects, "/repo").map(({ id, label }) => ({ id, label })),
    [
      { id: "default", label: "Default" },
      { id: "existing:newer", label: "Frontend" },
      { id: "existing:older", label: "Docs" },
      { id: "new", label: "New" },
    ],
  )
})

test("project selector cycles and serializes all launch policies", () => {
  const projects = [project("named", "Frontend", "named", "/repo", 100)]
  const existing = existingProjectSelectionId("named")

  assert.equal(cycleWaitingRoomProjectSelectionId("default", projects, 1, "/repo"), existing)
  assert.equal(cycleWaitingRoomProjectSelectionId(existing, projects, 1, "/repo"), "new")
  assert.equal(cycleWaitingRoomProjectSelectionId("new", projects, 1, "/repo"), "default")
  assert.equal(describeWaitingRoomProjectSelection(existing, projects, "/repo"), "Frontend")
  assert.deepEqual(projectSelectionFromId("default"), { kind: "default" })
  assert.deepEqual(projectSelectionFromId(existing), { kind: "existing", project_id: "named" })
  assert.deepEqual(projectSelectionFromId("new"), { kind: "new" })
})

function project(
  id: string,
  name: string,
  kind: "default" | "named",
  workspaceId: string,
  activityAt: number,
): WaitingRoomProjectSummary {
  return {
    id,
    owner_user_id: "owner",
    workspace_id: workspaceId,
    name,
    kind,
    status: "active",
    created_at_ms: 1,
    updated_at_ms: activityAt,
    session_count: 0,
    last_session_activity_at_ms: activityAt,
    joined_collaborator_count: 0,
    pending_collaboration_invite_count: 0,
  }
}
