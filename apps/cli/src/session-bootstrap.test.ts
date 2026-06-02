import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { hydrateOutlineAgentEntries } from "./session-history-outline.js"

test("bootstrapSession returns waiting-room bootstrap when no session should attach", async () => {
  const catalog = fallbackProviderCatalog()
  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      model: "default",
      accountProfile: "default",
      effort: "",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => { throw new Error("should not resolve") },
      attachToSession: async () => { throw new Error("should not attach") },
      getSessionState: async () => { throw new Error("should not fetch") },
      launchProviderRun: async () => { throw new Error("should not launch") },
      tryGetProviderRun: async () => { throw new Error("should not lookup") },
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(bootstrap.binding, null)
  assert.deepEqual(bootstrap.sessions, [])
  assert.equal(bootstrap.providerCatalog, catalog)
  assert.deepEqual(bootstrap.providerCommandCatalogs, fallbackProviderCommandCatalogs())
})

test("bootstrapSession attaches, launches, and hydrates history for the visible agent", async () => {
  const catalog = fallbackProviderCatalog()
  const launched: Array<{ provider: string; model: string; effort: string }> = []
  const session = {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 8,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-1",
      alias: null,
      provider: "codex",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      worktree_id: null,
      state: "Idle" as const,
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }
  const calls: string[] = []
  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      model: "gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [session],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => {
        calls.push("resolve")
        return session
      },
      attachToSession: async () => {
        calls.push("attach")
        return { id: "attachment-1", session_id: "session-1" }
      },
      getSessionState: async () => {
        calls.push("session")
        return session
      },
      launchProviderRun: async (_client, _sessionId, provider, _accountProfile, model, effort) => {
        calls.push("launch")
        launched.push({ provider, model, effort })
        return {
          id: "run-1",
          session_id: "session-1",
          agent_instance_id: "agent-a",
          adapter_key: provider,
          provider,
          account_profile: "default",
          model,
          variant: effort,
          usage_tokens_total: null,
          state: "Running",
        }
      },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => {
        calls.push("catchup")
      },
      getSessionHistoryOutline: async (_client, _sessionId, agentIds) => {
        calls.push(`outline:${agentIds.join(",")}`)
        return {
          agents: [outlineAgent("agent-a", "hi", "done")],
        }
      },
      getPromptInputHistory: async () => {
        calls.push("prompt-history")
        return {
          entries: [{
            sequence: 1,
            timestamp_ms: 1,
            session_id: "session-1",
            kind: "prompt",
            text: "hi",
          }],
        }
      },
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryOutlineAgent: (agent) => agent.turns.map((_turn, index) => ({ id: index + 1, role: "user", text: "hi" })),
    },
  )

  assert.deepEqual(calls, ["resolve", "attach", "session", "launch", "catchup", "session", "outline:agent-a", "prompt-history"])
  assert.deepEqual(launched, [{ provider: "codex", model: "codex/gpt-5.4-mini", effort: "low" }])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-1")
  assert.equal(bootstrap.binding?.providerRun?.id, "run-1")
  assert.deepEqual(bootstrap.binding?.historyEntries, [])
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.historyEntries, [{ id: 1, role: "user", text: "hi" }])
  assert.deepEqual(deferredHistory?.agentEntries["agent-a"], [{ id: 1, role: "user", text: "hi" }])
  assert.deepEqual(deferredHistory?.promptHistoryEntries, ["hi"])
})

test("bootstrapSession reattaches and hydrates missed output from history catch-up", async () => {
  const catalog = fallbackProviderCatalog()
  const session = {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: "run-1",
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-a",
    max_agents: 8,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-1",
      alias: null,
      provider: "opencode",
      model: "gpt-5.4",
      worktree_id: null,
      state: "Idle" as const,
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }
  const calls: string[] = []

  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      model: "gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [session],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => session,
      attachToSession: async () => {
        calls.push("attach")
        return { id: "attachment-2", session_id: "session-1" }
      },
      getSessionState: async () => {
        calls.push("session")
        return session
      },
      launchProviderRun: async () => { throw new Error("should not relaunch") },
      tryGetProviderRun: async () => {
        calls.push("load-run")
        return {
          id: "run-1",
          session_id: "session-1",
          agent_instance_id: "agent-a",
          adapter_key: "opencode",
          provider: "opencode",
          account_profile: "default",
          model: "gpt-5.4",
          variant: "high",
          usage_tokens_total: null,
          state: "Running",
        }
      },
      catchUpAttachedSession: async () => {
        calls.push("catchup")
      },
      getSessionHistoryOutline: async () => ({
        agents: [outlineAgent("agent-a", "hello\n", "while you were away")],
      }),
      getPromptInputHistory: async () => ({
        entries: [{
          sequence: 1,
          timestamp_ms: 1,
          session_id: "session-1",
          kind: "prompt",
          text: "hello\n",
        }],
      }),
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryOutlineAgent: hydrateOutlineAgentEntries,
    },
  )

  assert.deepEqual(calls, ["attach", "session", "load-run", "catchup", "session"])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-2")
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.promptHistoryEntries, ["hello"])
  const assistantEntry = deferredHistory?.historyEntries.find((entry) => entry.role === "assistant")
  assert.equal(assistantEntry?.text, "while you were away")
})

test("bootstrapSession skips attach-time launch when focused agent is stale", async () => {
  const warnings: Array<Record<string, unknown> | undefined> = []
  const session = {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "missing-agent",
    max_agents: 8,
    agents: [{
      id: "agent-a",
      agent_ref: "agent-a",
      session_id: "session-1",
      alias: null,
      provider: "codex",
      model: "codex/gpt-5.4-mini",
      effort: "low",
      worktree_id: null,
      state: "Idle" as const,
      is_processing: false,
      grid_row: 0,
      grid_col: 0,
      grid_row_span: 1,
      grid_col_span: 1,
      created_at_ms: 1,
      last_activity_at_ms: 1,
    }],
    config_state: { version: 1, values: {} },
  }

  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      model: "gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      logger: {
        warn: (_message: string, fields?: Record<string, unknown>) => warnings.push(fields),
      } as never,
      listSessions: async () => [session],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => session,
      attachToSession: async () => ({ id: "attachment-1", session_id: "session-1" }),
      getSessionState: async () => session,
      launchProviderRun: async () => { throw new Error("should not launch with stale focus") },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(bootstrap.binding?.providerRun, null)
  assert.deepEqual(warnings, [{
    session_id: "session-1",
    focused_agent_id: "missing-agent",
  }])
})

function outlineAgent(agentId: string, prompt: string, summary: string) {
  return {
    agent_id: agentId,
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      user_prompt: {
        entry_index: 1,
        fragment_start: 0,
        fragment_end: prompt.length,
        total_chars: prompt.length,
        entry: { kind: "user_prompt" as const, text: prompt, agent_id: agentId },
      },
      entries: [],
      summary: {
        entry_index: 2,
        fragment_start: 0,
        fragment_end: summary.length,
        total_chars: summary.length,
        entry: { kind: "provider_output" as const, text: summary, agent_id: agentId },
      },
      blobs: [],
    }],
    next_cursor: null,
  }
}
