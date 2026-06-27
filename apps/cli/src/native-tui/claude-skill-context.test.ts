import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, RuntimeSession } from "../cli-types.js"
import type { LocalIpcClient } from "../ipc.js"
import { buildClaudeNativeSkillContext } from "./claude-skill-context.js"

test("Claude native skill context reads session state with projected activity", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetSessionState" in request) {
        return {
          SessionState: {
            session: runtimeSession({
              agents: [{
                ...agent("agent-1"),
                extension_grants: [{ kind: "skill", name: "reviewer" }],
              }],
            }),
            agent_activity: {
              "agent-1": {
                status: "working",
                prompt_status: "running",
                busy: true,
              },
            },
            agent_activity_revision: 4,
          },
        }
      }
      return {
        Skill: {
          skill: {
            name: "reviewer",
            description: "Review changes",
            short_description: "Review code",
            path: "/unused/SKILL.md",
          },
        },
      }
    },
  } as unknown as LocalIpcClient

  const context = await buildClaudeNativeSkillContext(client, "session-1", "/workspace", "agent-1", "check this")

  assert.match(context, /Available Arroba skills/)
  assert.match(context, /`reviewer`: Review code/)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["GetSessionState", "GetSkill"])
})

function runtimeSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
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
    agents: [agent("agent-1")],
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

function agent(id: string): AgentInstance {
  return {
    id,
    agent_ref: id,
    session_id: "session-1",
    alias: id,
    provider: "claude",
    model: "sonnet-4.6",
    worktree_id: "/workspace",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
}
