import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { hydrateTranscriptEntries } from "./transcript-history.js"

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
      getSessionHistory: async () => ({ entries: [], next_cursor: null }),
      resolveVisibleAgentId: () => null,
      prepareHistoryEntries: () => [],
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
      getSessionHistory: async (_client, _sessionId, _cursor, agentId) => {
        calls.push(`history:${agentId}`)
        return {
          entries: [{
            entry_index: 0,
            fragment_start: 0,
            fragment_end: 1,
            total_chars: 1,
            entry: { kind: "user_prompt", text: "hi" },
          }],
          next_cursor: null,
        }
      },
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryEntries: (entries) => entries.map((_entry, index) => ({ id: index + 1, role: "user", text: "hi" })),
    },
  )

  assert.deepEqual(calls, ["resolve", "attach", "session", "launch", "catchup", "session", "history:agent-a", "history:null"])
  assert.deepEqual(launched, [{ provider: "codex", model: "codex/gpt-5.4-mini", effort: "low" }])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-1")
  assert.equal(bootstrap.binding?.providerRun?.id, "run-1")
  assert.deepEqual(bootstrap.binding?.historyEntries, [])
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.historyEntries, [{ id: 1, role: "user", text: "hi" }])
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
      getSessionHistory: async (_client, _sessionId, _cursor, agentId) => ({
        entries: agentId === null
          ? [{
            entry_index: 3,
            fragment_start: 0,
            fragment_end: 6,
            total_chars: 6,
            entry: { kind: "user_prompt", text: "hello\n" },
          }]
          : [
            {
              entry_index: 4,
              fragment_start: 24,
              fragment_end: 42,
              total_chars: 42,
              entry: { kind: "provider_output", text: "while you were away", merge_key: "reply-1" },
            },
          ],
        next_cursor: null,
      }),
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryEntries: hydrateTranscriptEntries,
    },
  )

  assert.deepEqual(calls, ["attach", "session", "load-run", "catchup", "session"])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-2")
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.promptHistoryEntries, ["hello"])
  assert.equal(deferredHistory?.historyEntries[0]?.role, "assistant")
  assert.equal(deferredHistory?.historyEntries[0]?.text, "while you were away")
  assert.equal(deferredHistory?.historyEntries[0]?.historyDeferred, true)
})
