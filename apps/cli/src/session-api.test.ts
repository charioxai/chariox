import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { createSession } from "./session-api.js"

test("createSession forwards workspace live sync mode to the kernel request", async () => {
  const sent: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      sent.push(request)
      return { SessionCreated: { session: runtimeSession({ workspace_live_sync_mode: "managed" }) } }
    },
  } as unknown as LocalIpcClient

  const session = await createSession(
    client,
    "/workspace",
    "/workspace",
    undefined,
    undefined,
    null,
    "managed",
  )

  assert.deepEqual(sent, [{
    CreateSession: {
      workspace_id: "/workspace",
      worktree_id: "/workspace",
      alias: null,
      slice_ref: null,
      workspace_live_sync_mode: "managed",
    },
  }])
  assert.equal(session.workspace_live_sync_mode, "managed")
})

test("createSession forwards worker kernel placement to the kernel request", async () => {
  const sent: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      sent.push(request)
      return { SessionCreated: { session: runtimeSession() } }
    },
  } as unknown as LocalIpcClient

  await createSession(
    client,
    "/workspace",
    "/workspace",
    undefined,
    undefined,
    null,
    "off",
    "kernel-worker",
  )

  assert.deepEqual(sent, [{
    CreateSession: {
      workspace_id: "/workspace",
      worktree_id: "/workspace",
      alias: null,
      slice_ref: null,
      kernel_ref: "kernel-worker",
      workspace_live_sync_mode: "unrestricted",
    },
  }])
})

function runtimeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "opencode",
      model: "gpt-5.4",
      effort: "medium",
      account_profile: null,
      execution_mode: "build",
      permission_level: "yolo",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 6,
    agents: [],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}
