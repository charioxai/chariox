import assert from "node:assert/strict"
import test from "node:test"

import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
import { bootstrapWaitingRoom } from "./waiting-room-bootstrap.js"
import { bootstrapSession } from "./session-bootstrap.js"
import { hydrateSessionHistoryOutlineAgentEntries } from "@chariox/kernel-client/session-history-transcript"
import type { CliOptions, RuntimeSession } from "./cli-types.js"
import { LocalIpcError } from "./ipc.js"

function terminalCatalog() {
  return {
    revision: "test",
    nodes: [],
  }
}

test("MP-08 / MP-10: initial-view bootstrap uses bundled history and preserves buffered live output", async () => {
  const session = { id: "s", active_provider_run_id: "run-1", agents: [{ id: "a" }], focused_agent_id: "a", workspace_id: "/workspace", worktree_id: "/workspace" }
  const outline = { agents: [outlineAgent("a", "real earlier prompt", "earlier answer")] }
  const unexpected = async () => { throw new Error("extra attach/state/history or destructive catch-up") }
  let bulkReads = 0
  const pendingPromptHistory = new Promise<never>(() => {})
  const boot = await bootstrapSession({} as never, { clientId: "cli", sessionId: "s", model: "known", effort: "high", provider: "codex", accountProfile: "default" }, "/workspace", "/workspace", {}, {
    listSessions: async () => [session], resolveSession: async () => session,
    attachSessionView: async () => ({ attachment: { id: "binding", session_id: "s" }, session, history: outline }),
    attachToSession: unexpected, getSessionState: unexpected, catchUpAttachedSession: unexpected,
    getSessionHistoryOutline: unexpected, tryGetProviderRun: async () => null,
    getPromptInputHistory: () => { bulkReads++; return pendingPromptHistory },
    getProviderCatalog: async () => { bulkReads++; return fallbackProviderCatalog() },
    getProviderCommandCatalogs: async () => { bulkReads++; return fallbackProviderCommandCatalogs() },
    getTerminalCommandCatalog: async () => { bulkReads++; return terminalCatalog() },
    resolveVisibleAgentId: () => "a", prepareHistoryOutlineAgent: hydrateSessionHistoryOutlineAgentEntries,
  } as never)
  assert.equal(bulkReads, 0, "MP-08 / MP-10: attach must not request composer inventories")
  assert.equal(boot.binding?.attachment.id, "binding")
  const history = await Promise.race([boot.deferred?.attachedHistory, new Promise<"blocked">((resolve) => setTimeout(() => resolve("blocked"), 20))])
  assert.notEqual(history, "blocked", "bundled transcript must not wait for composer input history")
  if (history === "blocked") return
  assert.equal(history?.agentEntries.a?.length, 2)
})

test("MP-08 / MP-10 / MP-11: explicit attach mounts without startup inventory or launch defaults", async () => {
  const session = { id: "s", active_provider_run_id: "run", agents: [{ id: "a" }], focused_agent_id: "a" }
  const calls: string[] = []
  const pending = new Promise<never>(() => {})
  const unexpected = async () => { throw new Error("unexpected fallback") }
  const boot = bootstrapSession({} as never, { clientId: "cli", sessionId: "alias", model: "default", effort: "", accountProfile: "default" }, "/workspace", "/workspace", {}, {
    getConfiguredProviderLaunchDefaults: () => { calls.push("defaults"); return pending },
    listSessions: () => { calls.push("inventory"); return pending },
    resolveSession: async () => { calls.push("resolve"); return session },
    attachSessionView: async () => { calls.push("view"); return { attachment: { id: "binding", session_id: "s" }, session, history: { agents: [outlineAgent("a", "prompt", "answer")] } } },
    attachToSession: unexpected, getSessionState: unexpected, catchUpAttachedSession: unexpected,
    getSessionHistoryOutline: unexpected, tryGetProviderRun: async () => null,
    getProviderCatalog: async () => fallbackProviderCatalog(),
    getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(),
    resolveVisibleAgentId: () => "a", prepareHistoryOutlineAgent: hydrateSessionHistoryOutlineAgentEntries,
  } as never)
  const result = await Promise.race([boot, new Promise<"blocked">(resolve => setTimeout(() => resolve("blocked"), 30))])
  assert.notEqual(result, "blocked", "explicit attach waited for unrelated startup reads")
  assert.deepEqual(calls, ["resolve", "view"])
  if (result !== "blocked") {
    assert.equal((await result.deferred?.attachedHistory)?.historyEntries.length, 2)
    assert.deepEqual(result.sessions.map(session => session.id), ["s"])
  }
})

test("MP-08 / MP-10: the bundled later desk hydrates before any off-desk outline request", async () => {
  const session = { id: "s", active_provider_run_id: "run", agents: Array.from({ length: 8 }, (_, i) => ({ id: `a${i}` })), focused_agent_id: "a7" }
  const outline = { agents: [outlineAgent("a6", "six", "six answer"), outlineAgent("a7", "seven", "seven answer")] }
  let extraRequests = 0
  const unexpected = async () => { throw new Error("extra attach/state/catch-up") }
  const boot = await bootstrapSession({} as never, { clientId: "cli", sessionId: "s", model: "known", effort: "high", provider: "codex", accountProfile: "default" }, "/workspace", "/workspace", {}, {
    listSessions: async () => [session], resolveSession: async () => session,
    attachSessionView: async () => ({ attachment: { id: "binding", session_id: "s" }, session, history: outline }),
    attachToSession: unexpected, getSessionState: unexpected, catchUpAttachedSession: unexpected,
    getSessionHistoryOutline: () => { extraRequests++; return new Promise<never>(() => {}) }, tryGetProviderRun: async () => null,
    getProviderCatalog: async () => fallbackProviderCatalog(), getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(), getTerminalCommandCatalog: async () => terminalCatalog(),
    resolveVisibleAgentId: () => "a7", prepareHistoryOutlineAgent: hydrateSessionHistoryOutlineAgentEntries,
  } as never)
  const history = await Promise.race([boot.deferred?.attachedHistory, new Promise<"blocked">(resolve => setTimeout(() => resolve("blocked"), 20))])
  assert.notEqual(history, "blocked")
  assert.equal(extraRequests, 0)
  if (history !== "blocked") assert.deepEqual(Object.keys(history?.agentEntries ?? {}), ["a6", "a7"])
})

test("waiting-room bootstrap mounts while kernel reports are pending", async () => {
  const pending = new Promise<never>(() => {})
  const boot = bootstrapSession({} as never, { clientId: "cli", model: "known", effort: "high", provider: "codex", accountProfile: "default" }, "/workspace", "/workspace", {}, {
    listSessions: () => pending,
    getProviderCatalog: () => pending,
    getProviderCommandCatalogs: () => pending,
    getTerminalCommandCatalog: () => pending,
  } as never)
  const result = await Promise.race([boot, new Promise<"blocked">((resolve) => setTimeout(() => resolve("blocked"), 20))])
  assert.notEqual(result, "blocked")
  if (result !== "blocked") assert.equal(result.binding, null)
})

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
      getTerminalCommandCatalog: async () => terminalCatalog(),
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
  assert.equal(await bootstrap.deferred?.providerCatalog, catalog)
  assert.deepEqual(await bootstrap.deferred?.providerCommandCatalogs, fallbackProviderCommandCatalogs())
})

test("bootstrapSession seeds provider/model/effort from the kernel config.toml default when unset", async () => {
  const catalog = fallbackProviderCatalog()
  const options: CliOptions = {
    clientId: "cli-1",
    model: "default",
    accountProfile: "default",
    effort: "",
  }

  const bootstrap = await bootstrapSession(
    {} as never,
    options,
    "/workspace",
    "/workspace",
    {},
    {
      getConfiguredProviderLaunchDefaults: async () => ({
        provider: "codex",
        model: "gpt-5.1",
        effort: "high",
      }),
      listSessions: async () => [],
      getProviderCatalog: async () => catalog,
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
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

  assert.deepEqual(await bootstrap.deferred?.waitingRoomDefaults, { provider: "codex", model: "gpt-5.1", effort: "high" })
  assert.equal(options.provider, undefined)
  assert.equal(options.model, "default")
  assert.equal(options.effort, "")
})

test("bootstrapSession attaches, launches, and hydrates history for the visible agent", async () => {
  const catalog = fallbackProviderCatalog()
  const launched: Array<{ provider: string; model: string; effort: string }> = []
  const session = {
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
      getTerminalCommandCatalog: async () => terminalCatalog(),
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
          agents: [outlineAgent("agent-a", "hi", "done", { before_sequence: 9 })],
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

  assert.deepEqual(calls, ["resolve", "attach", "session", "launch", "catchup", "session", "outline:agent-a"])
  assert.deepEqual(launched, [{ provider: "codex", model: "codex/gpt-5.4-mini", effort: "low" }])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-1")
  assert.equal(bootstrap.binding?.providerRun?.id, "run-1")
  assert.deepEqual(bootstrap.binding?.historyEntries, [])
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.historyEntries, [{ id: 1, role: "user", text: "hi" }])
  assert.deepEqual(deferredHistory?.agentEntries["agent-a"], [{ id: 1, role: "user", text: "hi" }])
  assert.deepEqual(deferredHistory?.promptHistoryEntries, [])
  assert.deepEqual((await bootstrap.deferred?.loadPromptInputHistory?.(bootstrap.binding!.session.id))?.entries, ["hi"])
  assert.deepEqual(deferredHistory?.nextHistoryCursor, {
    agentId: "agent-a",
    cursor: { before_sequence: 9 },
  })
})

test("bootstrapSession seeds newly created sessions from the requested provider account selection", async () => {
  const launched: Array<{ provider: string; model: string; effort: string; agentId: string | null | undefined }> = []
  let createdAgentDefaults: RuntimeSession["agent_defaults"] | undefined
  const createdSession = {
    id: "session-created",
    project_id: "project-default",
    workspace_id: "/workspace",
    worktree_id: "/workspace",
    created_at_ms: 1,
    status: "Active",
    agent_defaults: {
      provider: "opencode",
      model: "opencode/gpt-5.4",
      effort: "high",
      account_profile: "default",
    },
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 8,
    agents: [],
    config_state: { version: 1, values: {} },
  }

  await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      createSession: true,
      provider: "opencode",
      model: "opencode/gpt-5.4",
      accountProfile: "default",
      effort: "high",
    },
    "/workspace",
    "/workspace",
    {},
    {
      listSessions: async () => [],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async (_client, _workspace, _worktree, _alias, agentDefaults) => {
        assert.ok(agentDefaults)
        createdAgentDefaults = agentDefaults
        return {
          ...createdSession,
          agent_defaults: agentDefaults,
        }
      },
      resolveSession: async () => { throw new Error("should not resolve") },
      attachToSession: async () => ({ id: "attachment-created", session_id: "session-created" }),
      getSessionState: async () => createdSession,
      launchProviderRun: async (_client, _sessionId, provider, _accountProfile, model, effort, agentId) => {
        launched.push({ provider, model, effort, agentId })
        return {
          id: "run-created",
          session_id: "session-created",
          agent_instance_id: agentId ?? null,
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
      catchUpAttachedSession: async () => undefined,
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => null,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.deepEqual(createdAgentDefaults, {
    provider: "opencode",
    model: "opencode/gpt-5.4",
    effort: "high",
    account_profile: "default",
  })
  assert.deepEqual(launched, [{
    provider: "opencode",
    model: "opencode/gpt-5.4",
    effort: "high",
    agentId: null,
  }])
})

test("bootstrapSession reattaches and hydrates missed output from history catch-up", async () => {
  const catalog = fallbackProviderCatalog()
  const session = {
    id: "session-1",
    project_id: "project-default",
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
      getTerminalCommandCatalog: async () => terminalCatalog(),
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
      prepareHistoryOutlineAgent: hydrateSessionHistoryOutlineAgentEntries,
    },
  )

  assert.deepEqual(calls, ["attach", "session", "load-run", "catchup", "session"])
  assert.equal(bootstrap.binding?.attachment.id, "attachment-2")
  assert.deepEqual(bootstrap.binding?.promptHistoryEntries, [])
  const deferredHistory = await bootstrap.deferred?.attachedHistory
  assert.deepEqual(deferredHistory?.promptHistoryEntries, [])
  assert.deepEqual((await bootstrap.deferred?.loadPromptInputHistory?.(bootstrap.binding!.session.id))?.entries, ["hello"])
  const assistantEntry = deferredHistory?.historyEntries.find((entry) => entry.role === "assistant")
  assert.equal(assistantEntry?.text, "while you were away")
})

test("bootstrapSession skips attach-time launch when focused agent is stale", async () => {
  const warnings: Array<Record<string, unknown> | undefined> = []
  const session = {
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
      getTerminalCommandCatalog: async () => terminalCatalog(),
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

test("bootstrapSession recovers when the launch target moves remote during attach", async () => {
  const agent = {
    id: "agent-a",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: null,
    provider: "claude",
    model: "sonnet",
    effort: "medium",
    worktree_id: null,
    state: "Idle" as const,
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
  }
  const localSession: RuntimeSession = {
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
    focused_agent_id: agent.id,
    max_agents: 8,
    agents: [agent],
    config_state: { version: 1, values: {} },
  }
  const remoteSession: RuntimeSession = {
    ...localSession,
    agents: [{
      ...agent,
      remote_execution: {
        worker_kernel_id: "worker-1",
        worker_machine_id: "machine-1",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }],
  }
  const launchError = new Error("agent became remote-backed")
  const info: Array<{ message: string; fields: Record<string, unknown> | undefined }> = []
  let stateReads = 0

  const bootstrap = await bootstrapSession(
    {} as never,
    {
      clientId: "cli-1",
      sessionId: "session-1",
      provider: "claude",
      model: "sonnet",
      accountProfile: "default",
      effort: "medium",
    },
    "/workspace",
    "/workspace",
    {},
    {
      logger: {
        info: (message: string, fields?: Record<string, unknown>) => info.push({ message, fields }),
      } as never,
      listSessions: async () => [localSession],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => localSession,
      attachToSession: async () => ({ id: "attachment-1", session_id: "session-1" }),
      getSessionState: async () => {
        stateReads += 1
        return stateReads === 1 ? localSession : remoteSession
      },
      launchProviderRun: async (_client, _sessionId, _provider, _accountProfile, _model, _effort, agentId) => {
        assert.equal(agentId, agent.id)
        throw launchError
      },
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => {},
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => agent.id,
      prepareHistoryOutlineAgent: () => [],
    },
  )

  assert.equal(bootstrap.binding?.providerRun, null)
  assert.deepEqual(info, [{
    message: "recovered attach-time provider launch after agent moved remote",
    fields: {
      session_id: "session-1",
      agent_id: "agent-a",
      worker_kernel_id: "worker-1",
    },
  }])
})

test("MP-08 / MP-10 PANE-03: a slow attach-time provider launch does not hold the first transcript view", async () => {
  const session = { id: "s", active_provider_run_id: null, agents: [{ id: "a", provider: "codex", model: "m", effort: "low" }], focused_agent_id: "a" }
  const launched = { id: "run-1", session_id: "s", agent_instance_id: "a", provider: "codex", model: "m", variant: "low" }
  let finishLaunch: (run: typeof launched) => void = () => {}
  const launch = new Promise<typeof launched>(resolve => { finishLaunch = resolve })
  const unexpected = async () => { throw new Error("extra attach/state/history or destructive catch-up") }
  let stateReads = 0
  const boot = bootstrapSession({} as never, { clientId: "cli", sessionId: "s", model: "m", effort: "low", provider: "codex", accountProfile: "default" }, "/workspace", "/workspace", {}, {
    listSessions: async () => [session], resolveSession: async () => session,
    attachSessionView: async () => ({ attachment: { id: "binding", session_id: "s" }, session, history: { agents: [outlineAgent("a", "earlier prompt", "earlier answer")] } }),
    attachToSession: unexpected, catchUpAttachedSession: unexpected, getSessionHistoryOutline: unexpected,
    getSessionState: async () => { stateReads++; return { ...session, active_provider_run_id: "run-1" } },
    launchProviderRun: () => launch, tryGetProviderRun: async () => null,
    getProviderCatalog: async () => fallbackProviderCatalog(), getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(), getTerminalCommandCatalog: async () => terminalCatalog(),
    resolveVisibleAgentId: () => "a", prepareHistoryOutlineAgent: hydrateSessionHistoryOutlineAgentEntries,
  } as never)
  const first = await Promise.race([boot, new Promise<"blocked">(resolve => setTimeout(() => resolve("blocked"), 30))])
  assert.notEqual(first, "blocked", "the first view waited for the provider launch")
  if (first === "blocked") return
  assert.equal((await first.deferred?.attachedHistory)?.historyEntries.length, 2)
  assert.equal(first.binding?.providerRun, null)
  finishLaunch(launched)
  const settled = await first.deferred?.attachLaunch
  assert.equal(settled?.providerRun?.id, "run-1")
  assert.equal(settled?.session?.active_provider_run_id, "run-1")
  assert.equal(stateReads, 1)
})

function outlineAgent(
  agentId: string,
  prompt: string,
  summary: string,
  nextCursor: { before_sequence: number } | null = null,
) {
  return {
    agent_id: agentId,
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      lifecycle: "completed",
      completed_at_ms: 2,
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
    next_cursor: nextCursor,
  }
}


test("MP-08/MP-11 delayed defaults survive fallback projection during focus navigation", async () => {
  let release!: (value: {provider: string; model: string; effort: string}) => void
  const configured = new Promise<{provider: string; model: string; effort: string}>(resolve => { release = resolve })
  const options: CliOptions = {clientId: "cli", model: "default", effort: "", accountProfile: "default"}
  const boot = bootstrapWaitingRoom({} as never, options, {}, {
    getConfiguredProviderLaunchDefaults: () => configured,
    getProviderCatalog: async () => fallbackProviderCatalog(),
    getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(),
  } as never)
  // Reconciliation writes projected fallback values without a user-choice revision.
  options.provider = "opencode"
  options.model = "opencode/gpt-5.4"
  options.effort = "medium"
  release({provider: "codex", model: "gpt-6.1-sol", effort: "high"})
  assert.deepEqual(await boot.deferred?.waitingRoomDefaults, {provider: "codex", model: "gpt-6.1-sol", effort: "high"})
})

test("MP-08/MP-11 configured defaults hydrate while provider metadata is still pending", async () => {
  let releaseCatalog!: (value: ReturnType<typeof fallbackProviderCatalog>) => void
  const catalog = new Promise<ReturnType<typeof fallbackProviderCatalog>>(resolve => { releaseCatalog = resolve })
  const boot = bootstrapWaitingRoom({} as never, {clientId: "cli", model: "default", effort: "", accountProfile: "default"}, {}, {
    getConfiguredProviderLaunchDefaults: async () => ({provider: "codex", model: "codex/gpt-6.1-sol", effort: "high"}),
    getProviderCatalog: () => catalog,
    getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(),
  } as never)
  let hydrated: unknown = null
  void boot.deferred?.waitingRoomDefaults?.then(value => { hydrated = value })
  await new Promise(resolve => setImmediate(resolve))
  const beforeMetadata = hydrated
  releaseCatalog(fallbackProviderCatalog())
  await boot.deferred?.providerCatalog
  assert.deepEqual(beforeMetadata, {provider: "codex", model: "codex/gpt-6.1-sol", effort: "high"})
})


test("MP-08/MP-11 defaults recover after the first live report when relay routing drops", async () => {
  const { LocalIpcError } = await import("./ipc.js")
  let report!: (event: { event: string }) => void
  let disposed = 0, reads = 0
  const client = { onKernelEvent: (handler: typeof report) => { report = handler; return () => { disposed += 1 } } }
  const boot = bootstrapWaitingRoom(client as never, { clientId: "cli", model: "default", effort: "", accountProfile: "default" }, {}, {
    getConfiguredProviderLaunchDefaults: async () => {
      if (++reads === 1) throw new LocalIpcError("read configured defaults", "target daemon disconnected from relay", "target_disconnected", true)
      return { provider: "codex", model: "codex/gpt-6.1-sol", effort: "high" }
    },
    getProviderCatalog: async () => fallbackProviderCatalog(),
    getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(),
  } as never)
  let outcome = "pending"
  void boot.deferred?.waitingRoomDefaults?.then(() => { outcome = "ready" }, () => { outcome = "failed" })
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(outcome, "pending")
  report({ event: "waiting_room_rows_changed" })
  assert.deepEqual(await boot.deferred?.waitingRoomDefaults, { provider: "codex", model: "codex/gpt-6.1-sol", effort: "high" })
  assert.equal(reads, 2)
  assert.equal(disposed, 1)
})

test("MP-08 / MP-11 direct session creation checks the default account before creating work", async () => {
  let created = false
  await assert.rejects(bootstrapSession({} as never, {
    clientId: "cli-1", createSession: true, provider: "codex", model: "default", accountProfile: "default", effort: "",
  }, "/workspace", "/workspace", {}, {
    listProviderAccountProfiles: async () => [{ owner_user_id: "owner-1", provider: "codex", profile_id: "codex-1", label: "codex-1", origin: "default", is_default: true, auth_state: "not_configured", materializations: [], identity_summary: null, usage: { profile_id: "codex-1", provider: "codex", availability: "unavailable", meters: [], source: "test" } }],
    listSessions: async () => [],
    getProviderCatalog: async () => fallbackProviderCatalog(),
    getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(),
    createSession: async () => { created = true; throw new Error("must not create") },
    resolveSession: async () => { throw new Error("must not resolve") },
    attachToSession: async () => { throw new Error("must not attach") },
    getSessionState: async () => { throw new Error("must not fetch") },
    launchProviderRun: async () => { throw new Error("must not launch") },
    tryGetProviderRun: async () => null,
    catchUpAttachedSession: async () => undefined,
    getSessionHistoryOutline: async () => ({ agents: [] }),
    resolveVisibleAgentId: () => null,
    prepareHistoryOutlineAgent: () => [],
  }), /codex-1 needs sign-in/)
  assert.equal(created, false)
})

// After a kernel restart the Vault is locked: the kernel holds the attach-time launch on its own
// unlock prompt. The TUI must render (and show that prompt) instead of waiting blank for it.
test("bootstrapSession renders while the kernel holds the attach-time launch on a Vault unlock prompt", async () => {
  const agent = { id: "agent-a", agent_ref: "agent-a", session_id: "session-1", alias: null, provider: "claude-p", model: "claude-haiku-5-5", effort: "low",
    worktree_id: null, state: "Idle" as const, is_processing: false, grid_row: 0, grid_col: 0, grid_row_span: 1, grid_col_span: 1, created_at_ms: 1, last_activity_at_ms: 1 }
  const session = { id: "session-1", project_id: "project-default", workspace_id: "/workspace", worktree_id: "/workspace", created_at_ms: 1, status: "Active",
    active_provider_run_id: null, attachment_ids: [], active_prompt: null, queued_prompts: [], focused_agent_id: "agent-a", max_agents: 8, agents: [agent],
    config_state: { version: 1, values: {} } }
  const run = { id: "run-1", session_id: "session-1", agent_instance_id: "agent-a", adapter_key: "claude-p", provider: "claude-p", account_profile: "default",
    model: "claude-haiku-5-5", variant: "low", usage_tokens_total: null, state: "Running" }
  let unlock!: (value: typeof run) => void
  const pending = bootstrapSession({} as never, { clientId: "cli-1", sessionId: "session-1", model: "default", accountProfile: "default", effort: "low" },
    "/workspace", "/workspace", {}, {
      deferProviderLaunch: true,
      listSessions: async () => [session],
      getProviderCatalog: async () => fallbackProviderCatalog(),
      getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
      getTerminalCommandCatalog: async () => terminalCatalog(),
      createSession: async () => { throw new Error("should not create") },
      resolveSession: async () => session,
      attachToSession: async () => ({ id: "attachment-1", session_id: "session-1" }),
      getSessionState: async () => session,
      launchProviderRun: () => new Promise(resolve => { unlock = resolve }),
      tryGetProviderRun: async () => null,
      catchUpAttachedSession: async () => {},
      getSessionHistoryOutline: async () => ({ agents: [] }),
      resolveVisibleAgentId: () => "agent-a",
      prepareHistoryOutlineAgent: () => [],
    })
  const bootstrap = await Promise.race([pending, new Promise<"held">(resolve => setTimeout(() => resolve("held"), 1000))])
  assert.notEqual(bootstrap, "held", "the TUI renders while the kernel waits for the Vault passphrase")
  if (bootstrap === "held") return
  assert.equal(bootstrap.binding?.providerRun, null)
  const starting = bootstrap.deferred!.providerLaunch!.start()
  await new Promise(resolve => setImmediate(resolve))
  unlock(run)
  assert.equal((await starting).providerRun?.id, "run-1")
})


// MP-08/MP-10/MP-11 F02: a launch can require a human Vault answer.
test("scoped bootstrap renders its attachment before a protected provider launch", async () => {
  const session = { id: "room", agents: [{ id: "agent", provider: "claude-p", model: "claude-haiku-5-5", state: "Idle" }], focused_agent_id: "agent", active_provider_run_id: null }
  let launched = false
  const pending = new Promise<never>(() => {})
  const boot = bootstrapSession({} as never, { clientId: "cli", sessionId: "room", provider: "claude-p", model: "claude-haiku-5-5", effort: "low", accountProfile: "selected" }, "/workspace", "/workspace", {}, {
    deferProviderLaunch: true,
    listSessions: async () => [session], resolveSession: async () => session,
    attachToSession: async () => ({ id: "attachment", session_id: "room" }),
    getSessionState: async () => session,
    launchProviderRun: () => { launched = true; return pending },
    catchUpAttachedSession: async () => {}, getSessionHistoryOutline: async () => ({ agents: [] }),
    getProviderCatalog: async () => fallbackProviderCatalog(), getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(), resolveVisibleAgentId: () => "agent",
    prepareHistoryOutlineAgent: () => [],
  } as never)
  const result = await Promise.race([boot, new Promise<"blocked">((resolve) => setTimeout(() => resolve("blocked"), 30))])
  assert.notEqual(result, "blocked", "Vault input must not depend on an unmounted TUI")
  assert.equal(launched, false, "only the mounted client starts the protected launch")
  if (result !== "blocked") {
    assert.equal(result.binding?.session.id, "room")
    assert.ok(result.deferred?.providerLaunch)
    void result.deferred.providerLaunch.start()
    await new Promise<void>((resolve) => setTimeout(resolve, 0))
    assert.equal(launched, true)
  }
})

// MP-08/MP-10/MP-11: the deferred launch keeps the locked-vault reason for the mounted TUI.
test("bootstrapSession reports a locked credential vault from the deferred attach launch", async () => {
  const session = { id: "session-1", agents: [{ id: "agent-a", provider: "codex", model: "codex/gpt-5.4-mini", state: "Idle" }], focused_agent_id: "agent-a", active_provider_run_id: null }
  const bootstrap = await bootstrapSession({} as never, { clientId: "cli-1", sessionId: "session-1", model: "gpt-5.4", accountProfile: "default", effort: "high" }, "/workspace", "/workspace", {}, {
    deferProviderLaunch: true,
    listSessions: async () => [session], resolveSession: async () => session,
    attachToSession: async () => ({ id: "attachment-1", session_id: "session-1" }),
    getSessionState: async () => session,
    launchProviderRun: async () => {
      throw new LocalIpcError("handle kernel response", "local transport `credential_vault_locked` failed: Chariox vault is locked", "local_transport_error", true)
    },
    tryGetProviderRun: async () => null, catchUpAttachedSession: async () => undefined, getSessionHistoryOutline: async () => ({ agents: [] }),
    getProviderCatalog: async () => fallbackProviderCatalog(), getProviderCommandCatalogs: async () => fallbackProviderCommandCatalogs(),
    getTerminalCommandCatalog: async () => terminalCatalog(), resolveVisibleAgentId: () => null,
    prepareHistoryOutlineAgent: () => [],
  } as never)
  assert.equal(bootstrap.binding?.providerRun, null)
  assert.deepEqual(await bootstrap.deferred?.providerLaunch?.start(), { providerRun: null, issue: "credential_vault_locked" })
})

test("MP-08 / MP-10 / MP-11 full session IDs paint from the kernel bundle without a resolve round trip", async () => {
  const id = "18de8597bddf72dc", calls: string[] = []
  const session = {id,active_provider_run_id:"run",agents:[{id:"a"}],focused_agent_id:"a"}
  const unexpected = async () => { throw new Error("unnecessary startup read") }
  const boot = await bootstrapSession({} as never,{clientId:"cli",sessionId:id,model:"known",effort:"high",provider:"claude-p",accountProfile:"work"},"/workspace","/workspace",{}, {
    resolveSession: async () => { calls.push("resolve"); throw new Error("unnecessary resolve round trip") },
    attachSessionView: async (_client:unknown,sessionId:string,_clientId:string,preferred:string|null) => {calls.push("view");assert.equal(sessionId,id);assert.equal(preferred,null);return {attachment:{id:"binding",session_id:id},session,history:{agents:[outlineAgent("a","prompt","answer")]}}},
    listSessions:unexpected,attachToSession:unexpected,getSessionState:unexpected,catchUpAttachedSession:unexpected,getSessionHistoryOutline:unexpected,
    tryGetProviderRun:async()=>null,resolveVisibleAgentId:()=>"a",prepareHistoryOutlineAgent:hydrateSessionHistoryOutlineAgentEntries,
  } as never)
  assert.deepEqual(calls,["view"]);assert.equal(boot.binding?.session.id,id)
  assert.equal((await boot.deferred?.attachedHistory)?.historyEntries.length,2)
})
