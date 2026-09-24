import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import { createManagedSessionAfterProjectSetup } from "./cli-waiting-room-composition.js"
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

test("managed session creation does not return to provider attachment before Project setup is ready", async () => {
  const calls: string[] = []
  let releaseSetup = () => {}
  const setupGate = new Promise<void>((resolve) => {
    releaseSetup = resolve
  })
  const pending = createManagedSessionAfterProjectSetup({
    assertActive: () => {},
    createSession: async () => {
      calls.push("create session")
      return session()
    },
    prepareProject: async () => {
      calls.push("start Project setup")
      await setupGate
      calls.push("Project ready")
    },
    deleteSession: async () => {
      calls.push("delete session")
    },
    formatError: String,
  }).then((created) => {
    calls.push("provider attachment may start")
    return created
  })

  await Promise.resolve()
  await Promise.resolve()
  assert.deepEqual(calls, ["create session", "start Project setup"])

  releaseSetup()
  assert.equal((await pending).id, "session-1")
  assert.deepEqual(calls, [
    "create session",
    "start Project setup",
    "Project ready",
    "provider attachment may start",
  ])
})

test("cancelled managed Project setup removes the unlaunched session", async () => {
  const calls: string[] = []
  await assert.rejects(createManagedSessionAfterProjectSetup({
    assertActive: () => {},
    createSession: async () => {
      calls.push("create session")
      return session()
    },
    prepareProject: async () => {
      calls.push("setup cancelled")
      throw new Error("setup cancelled")
    },
    deleteSession: async () => {
      calls.push("delete session")
    },
    formatError: String,
  }), /setup cancelled/)
  assert.deepEqual(calls, ["create session", "setup cancelled", "delete session"])
})

function session(): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-1",
    workspace_id: "/managed/project",
    worktree_id: "/managed/project",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [{ id: "agent-1" } as RuntimeSession["agents"][number]],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
  }
}
