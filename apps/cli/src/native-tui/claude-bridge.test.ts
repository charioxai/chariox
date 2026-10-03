import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, PromptQueueItem, RuntimeSession } from "../cli-types.js"
import { sessionActivePromptForAgent } from "@chariox/kernel-client/session-prompt-identity"

test("native Claude bridge ignores stale prompts when projected activity is idle", () => {
  const stalePrompt = prompt("prompt-stale", "agent-1")
  const runtimeSession = session({
    active_prompt: stalePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: stalePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
  })

  assert.equal(sessionActivePromptForAgent(runtimeSession, "agent-1"), null)
})

test("native Claude bridge only uses prompt matching projected active turn", () => {
  const activePrompt = prompt("prompt-active", "agent-1")
  const stalePrompt = prompt("prompt-stale", "agent-1")
  const runtimeSession = session({
    active_prompt: stalePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: activePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-active",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.equal(sessionActivePromptForAgent(runtimeSession, "agent-1")?.id, "prompt-active")
  const projectedActivity = {
    "agent-1": {
      status: "working",
      prompt_status: "running",
      busy: true,
      unread_idle_output: false,
      active_turn: {
        prompt_id: "prompt-active",
        status: "running",
        phase: "streaming",
      },
    },
  } satisfies NonNullable<RuntimeSession["agent_activity"]>

  const mismatchedSession = session({
    active_prompt: stalePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: stalePrompt,
        queued_prompts: [],
      },
    },
    agent_activity: projectedActivity,
  })

  assert.equal(sessionActivePromptForAgent(mismatchedSession, "agent-1"), null)
})

test("native Claude bridge ignores unscoped legacy prompts before projected activity exists", () => {
  const activePrompt = prompt("prompt-legacy", "agent-1")

  assert.equal(sessionActivePromptForAgent(session({ active_prompt: activePrompt }), "agent-1"), null)
})

test("native Claude bridge ignores legacy active prompt once projected activity exists", () => {
  const activePrompt = prompt("prompt-stale", "agent-1")

  assert.equal(sessionActivePromptForAgent(session({
    active_prompt: activePrompt,
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
      },
    },
  }), "agent-1"), null)
})

test("native Claude bridge prefers explicit prompt state over stale top-level prompt", () => {
  const activePrompt = prompt("prompt-stale", "agent-1")

  assert.equal(sessionActivePromptForAgent(session({
    active_prompt: activePrompt,
    prompt_states: {},
  }), "agent-1"), null)
  assert.equal(sessionActivePromptForAgent(session({
    active_prompt: activePrompt,
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
  }), "agent-1"), null)
})

test("native Claude bridge does not inject queued prompts as active work", () => {
  assert.equal(sessionActivePromptForAgent(session({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [prompt("queued-1", "agent-1", "queued")],
      },
    },
  }), "agent-1"), null)
})

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "claude",
      model: "sonnet-4.6",
      effort: "medium",
      account_profile: null,
      execution_mode: "build",
      permission_level: "yolo",
    },
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
    alias: null,
    provider: "claude",
    model: "sonnet-4.6",
    effort: "medium",
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

function prompt(id: string, targetAgentId: string, status = "running"): PromptQueueItem {
  return {
    id,
    source_attachment_id: "attachment-1",
    target_agent_id: targetAgentId,
    prompt: "test prompt",
    status,
  }
}

// MP-08/MP-10: exercise the real bridge and generated UserPromptSubmit hook.
test("Claude Chariox-origin prompt carries hidden context only through the hook", { timeout: 3_000 }, async (t) => {
  const { mkdtemp, readFile, rm, writeFile } = await import("node:fs/promises")
  const { tmpdir } = await import("node:os")
  const { join } = await import("node:path")
  const { spawnSync } = await import("node:child_process")
  const { startClaudeBridge } = await import("./claude-bridge.js")
  const { writeClaudeHookHandler } = await import("./claude-hook-handler.js")
  const { hiddenInstructionsStart, hiddenInstructionsEnd } = await import("./hidden-instructions.js")
  const root = await mkdtemp(join(tmpdir(), "chariox-claude-bridge-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  const contextFile = join(root, "context.txt")
  const eventsFile = join(root, "events.jsonl")
  const handler = join(root, "hook.mjs")
  await writeFile(eventsFile, "")
  await writeClaudeHookHandler(handler)
  const hidden = "Fixture-only hidden instruction"
  const visible = "Reply with the fixture answer."
  const activePrompt = {
    ...prompt("prompt-external", "agent-1"),
    prompt: `${hiddenInstructionsStart}\n${hidden}\n${hiddenInstructionsEnd}\n\n${visible}`,
  }
  const state = session({
    prompt_states: { "agent-1": { active_prompt: activePrompt, queued_prompts: [] } },
    agent_activity: { "agent-1": {
      status: "working", prompt_status: "running", busy: true, unread_idle_output: false,
      active_turn: { prompt_id: activePrompt.id, status: "running", phase: "awaiting_first_output" },
    } },
  })
  const requests: unknown[] = []
  let finish: () => void = () => {}
  let fail: (error: unknown) => void = () => {}
  const submitted = new Promise<void>((resolve, reject) => { finish = resolve; fail = reject })
  const bridge = startClaudeBridge({
    client: { send: async (request: unknown) => {
      requests.push(request)
      return { SessionState: { session: state } }
    } } as unknown as import("../ipc.js").LocalIpcClient,
    sessionId: state.id, attachmentId: "attachment", agentId: "agent-1", providerRunId: "run",
    eventsFile, contextFile, originFile: join(root, "origin.json"), attachmentContextDir: join(root, "attachments"),
    hookContextResponseDir: join(root, "responses"), workspace: root, worktree: root,
    inlineLocalAttachments: false, promptOrigin: { current: null }, debug: () => {},
    submitPrompt: async (text) => {
      try {
        assert.equal(text, visible)
        assert.equal(await readFile(contextFile, "utf8"), hidden)
        const hook = spawnSync(process.execPath, [handler], {
          input: JSON.stringify({ hook_event_name: "UserPromptSubmit", prompt: text }),
          encoding: "utf8", timeout: 1_000,
          env: { ...process.env, CHARIOX_CLAUDE_NATIVE_EVENTS: eventsFile, CHARIOX_CLAUDE_NATIVE_CONTEXT: contextFile },
        })
        assert.equal(hook.status, 0)
        assert.deepEqual(JSON.parse(hook.stdout), {
          hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: hidden },
        })
      } catch (error) {
        fail(error)
      } finally {
        bridge.stop()
        finish()
      }
    },
  })
  t.after(() => bridge.stop())
  await submitted
  assert.ok(requests.every((request) => Object.keys(request as object)[0] === "GetSessionState"))
})
