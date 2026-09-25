import assert from "node:assert/strict"
import test from "node:test"

import { cliWaitingRoomSliceApiOptions } from "./waiting-room-slice-api-options.js"

test("production Waiting Room composition forwards slice Project development setup", () => {
  assert.deepEqual(cliWaitingRoomSliceApiOptions({
    name: "project-slice",
    displayMode: "headless",
    workspaceId: "/primary",
    worktreeId: "/primary-worktree",
    workspaceMount: "/primary-worktree",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [
        { role: "primary", workspaceId: "/primary", worktreeId: "/primary-worktree" },
        { role: "supporting", workspaceId: "/supporting", worktreeId: null },
      ],
    },
  }), {
    name: "project-slice",
    displayMode: "headless",
    workspaceId: "/primary",
    worktreeId: "/primary-worktree",
    workspaceMount: "/primary-worktree",
    developmentSetup: {
      kind: "source_project",
      projectId: "project-1",
      repositories: [
        { role: "primary", workspaceId: "/primary", worktreeId: "/primary-worktree" },
        { role: "supporting", workspaceId: "/supporting", worktreeId: null },
      ],
    },
  })
})
