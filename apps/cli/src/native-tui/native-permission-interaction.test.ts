import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "../cli-types.js"
import type { LocalIpcClient } from "../ipc.js"
import { resolveActiveNativePermissionInteraction } from "./native-permission-interaction.js"

test("native permission interaction reads session state with projected activity", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetSessionState" in request) {
        return {
          SessionState: {
            session: runtimeSession({
              active_interactions: [{
                id: "interaction-1",
                agent_id: "agent-1",
                kind: "permission",
                level: "warning",
                message: "Allow tool?",
                choices: [{ id: "approve", label: "Approve", reply: "approved" }],
                requested_at_ms: 1,
              }],
            }),
            agent_activity: {
              "agent-1": {
                status: "working",
                prompt_status: "running",
                busy: true,
              },
            },
            agent_activity_revision: 5,
          },
        }
      }
      return { InteractionResponded: { session: runtimeSession() } }
    },
  } as unknown as LocalIpcClient

  assert.equal(await resolveActiveNativePermissionInteraction(client, "session-1", "agent-1", "approve"), true)
  assert.deepEqual(requests, [
    { GetSessionState: { session_id: "session-1" } },
    { RespondToInteraction: { session_id: "session-1", interaction_id: "interaction-1", choice_id: "approve", custom_reply: null } },
  ])
})

function runtimeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
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
