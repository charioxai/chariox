import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import { createCommandActionHandlers, formatAgentCapabilityGrants, formatAgentListSummary, parseMcpInstallConfig, parseRequestedViewLayout } from "./command-actions.js"
import type { AgentInstance, ProviderProcessInfo, WorkflowQueuedPrompt, RuntimeAttachment, RuntimeProviderRun, RuntimeSession, WorkflowDefinition, WorkflowRun } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession, runGit } from "./command-actions-test-support.js"

test("agent command usage advertises hierarchical spawn placement", async () => {
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string) => {
      flashedMessage = message
    },
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent", args: [] })

  assert.match(flashedMessage, /--machine <machine-ref>\|--kernel <kernel-ref>\|--slice off\|new:headless\|new:headed\|<slice-ref>/)
  assert.doesNotMatch(flashedMessage, /--slice-display/)
})

test("agent task command updates focused metaagent task", async () => {
  const metaagent = makeAgent({ role: "meta", alias: "planner" })
  const nextSession = makeSession({
    agents: [metaagent],
    metaagent_tasks: [{
      task_id: "task-1",
      metaagent_id: metaagent.id,
      status: "active",
      task_markdown: "Fix tests",
      plan_markdown: "",
      revision: 1,
      created_at_ms: 1,
      updated_at_ms: 2,
    }],
  })
  const calls: string[] = []
  let appliedSession: RuntimeSession | null = null
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => makeSession({ agents: [metaagent], focused_agent_id: metaagent.id }),
    resolveSessionAgent: (reference?: string | null) => (
      !reference || reference === metaagent.id || reference === metaagent.agent_ref
        ? { agent: metaagent }
        : { agent: null, error: `agent '${reference}' not found` }
    ),
    updateMetaagentTask: async (_sessionId: string, metaagentId: string, updates: Record<string, unknown>) => {
      calls.push(`${metaagentId}:${updates.taskMarkdown}`)
      return nextSession
    },
    applySessionState: (session: RuntimeSession) => {
      appliedSession = session
    },
    flashFooter: (message: string) => {
      flashedMessage = message
    },
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent task edit Fix tests", args: ["task", "edit", "Fix", "tests"] })

  assert.deepEqual(calls, ["agent-1:Fix tests"])
  assert.equal(appliedSession, nextSession)
  assert.match(flashedMessage, /updated task for agent-1/)
})

test("agent task command rejects regular agents", async () => {
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    flashFooter: (message: string) => {
      flashedMessage = message
    },
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent task pause", args: ["task", "pause"] })

  assert.match(flashedMessage, /agent-1 is not a metaagent/)
})

test("agent spawn refreshes session state after launching the provider run", async () => {
  const firstAgent = makeAgent()
  const secondAgent = makeAgent({
    id: "agent-2",
    agent_ref: "agent-2",
    alias: "review",
    state: "Focused",
  })
  let currentSession = makeSession()
  const spawnedSession = makeSession({
    focused_agent_id: secondAgent.id,
    agents: [firstAgent, secondAgent],
  })
  const refreshedSession = {
    ...spawnedSession,
    active_provider_run_id: "provider-run-2",
  }
  const appliedProviderRunIds: Array<string | null> = []
  const refreshedPaneProviderRunIds: Array<string | null> = []
  let splitPaneRefreshCount = 0
  let flashedMessage = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => currentSession,
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun => ({
      id: "provider-run-1",
      session_id: "session-1",
      agent_instance_id: "agent-1",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "openai/gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "running",
    }),
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
    focusedAgentId: () => currentSession.focused_agent_id,
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    updateSessionConfig: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    applySessionState: (session) => {
      currentSession = session
      appliedProviderRunIds.push(session.active_provider_run_id)
    },
    refreshAgentPanes: async (session) => {
      refreshedPaneProviderRunIds.push(session.active_provider_run_id)
    },
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
    launchAgentProviderRun: async () => ({
      id: "provider-run-2",
      session_id: "session-1",
      agent_instance_id: "agent-2",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "openai/gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "running",
    }),
    setProviderRunState: () => {},
    refreshSessionState: async () => refreshedSession,
    spawnAgent: async () => ({ agent: secondAgent, session: spawnedSession }),
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: secondAgent, session: currentSession }),
    resolveSessionAgent: () => ({ agent: currentSession.agents[0] ?? null }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: "entry", entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => { splitPaneRefreshCount += 1 },
    formatSessionList: () => "",
  })

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review",
    args: ["spawn", "review"],
  })

  assert.deepEqual(appliedProviderRunIds, [null, "provider-run-2"])
  assert.deepEqual(refreshedPaneProviderRunIds, [null, "provider-run-2"])
  assert.equal(splitPaneRefreshCount, 1)
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · local · worktree worktree-1")
})

test("agent inspect renders diagnostics as a notice with concise footer", async () => {
  const agent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    alias: "qa",
    remote_execution: {
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
      active_worker_provider_run_id: "run-1",
    },
    extension_grants: [{ kind: "mcp", name: "filesystem" }],
    remote_extension_manifest_sync: {
      state: "stale",
      manifest_hash: "abcdef123456",
      last_error: "worker offline",
    },
  })
  let flashedMessage = ""
  const notices: string[] = []
  const handlers = createCommandActionHandlers(makeCommandDeps({
    sessionState: () => makeSession({ focused_agent_id: agent.id, agents: [agent] }),
    focusedAgentId: () => agent.id,
    resolveSessionAgent: () => ({ agent }),
    flashFooter: (message: string) => {
      flashedMessage = message
    },
    appendNotice: (message: string) => {
      notices.push(message)
    },
    listSlices: async () => [{
      id: "slice-1",
      name: "devbox",
      owner_kernel_id: "kernel-home",
      owner_machine_id: "machine-home",
      backend: "local_docker",
      os: "linux",
      status: "running",
      worktree_id: "worktree-1",
      worker_kernel_ref: "slice:slice-1",
      worker_kernel_id: "slice-kernel",
      worker_machine_id: "slice-machine",
      agent_ids: ["agent-remote"],
      created_at_ms: 0,
      updated_at_ms: 0,
    }],
    formatAgentLabel: (entry: AgentInstance | null | undefined) => entry?.agent_ref ?? "",
  }))

  await handlers.handleAgentCommand({ kind: "agent", raw: "/agent inspect", args: ["inspect"] })

  assert.equal(flashedMessage, "showing agent agent-remote")
  assert.equal(notices.length, 1)
  assert.match(notices[0] ?? "", /placement: slice devbox \(worker=slice-machine, kernel=slice-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=run-1\)/)
  assert.match(notices[0] ?? "", /slice: devbox \(id=slice-1, status=running, display=headless, worktree=worktree-1, agents=1\)/)
  assert.match(notices[0] ?? "", /remote extension sync: stale, hash=abcdef123456, error=worker offline/)
})

test("agent spawn count inherits session defaults for each spawn", async () => {
  const sourceAgent = makeAgent({
    id: "agent-source",
    agent_ref: "agent-source",
    provider: "opencode",
    model: "opencode/gpt-5.4",
  })
  let currentSession = makeSession({
    focused_agent_id: sourceAgent.id,
    agent_defaults: {
      provider: "codex",
      model: "codex/gpt-5.4",
      effort: "high",
      account_profile: "default",
      execution_mode: "build",
      permission_level: "yolo",
    },
    agents: [sourceAgent],
  })
  const spawnCalls: Array<{ provider: string | null | undefined; alias: string | undefined; model: string | null | undefined; effort: string | null | undefined }> = []
  const launchCalls: Array<{ provider: string; model: string; effort: string; agentId: string }> = []
  let refreshCount = 0
  let flashedMessage = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => currentSession,
    attachmentState: () => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: () => null,
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    focusedAgentId: () => currentSession.focused_agent_id,
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    updateSessionConfig: async () => ({ session: currentSession, config: currentSession.config_state }),
    applySessionState: (session) => {
      currentSession = session
    },
    refreshAgentPanes: async () => {
      refreshCount += 1
    },
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({ agent: null, session: currentSession }),
    launchAgentProviderRun: async (provider, model, effort, agentId) => {
      launchCalls.push({ provider, model, effort, agentId })
      return {
        id: `provider-run-${launchCalls.length}`,
        session_id: "session-1",
        agent_instance_id: agentId,
        adapter_key: provider,
        provider,
        account_profile: "default",
        model,
        variant: effort,
        usage_tokens_total: null,
        state: "running",
      }
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async (provider, alias, model, effort) => {
      spawnCalls.push({ provider, alias, model, effort })
      const resolvedProvider = provider ?? currentSession.agent_defaults?.provider ?? "opencode"
      const resolvedModel = model ?? currentSession.agent_defaults?.model ?? "test-model"
      const resolvedEffort = effort ?? currentSession.agent_defaults?.effort ?? "low"
      const agent = makeAgent({
        id: `agent-${spawnCalls.length}`,
        agent_ref: `agent-${spawnCalls.length}`,
        provider: resolvedProvider,
        model: resolvedModel,
        effort: resolvedEffort,
        state: "Focused",
      })
      currentSession = makeSession({
        agent_defaults: currentSession.agent_defaults!,
        focused_agent_id: agent.id,
        agents: [...currentSession.agents, agent],
      })
      return { agent, session: currentSession }
    },
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: currentSession.agents[0] ?? sourceAgent, session: currentSession }),
    resolveSessionAgent: () => ({ agent: currentSession.agents[0] ?? null }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: currentSession }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn 2",
    args: ["spawn", "2"],
  })

  assert.deepEqual(spawnCalls, [
    { provider: undefined, alias: undefined, model: undefined, effort: undefined },
    { provider: undefined, alias: undefined, model: undefined, effort: undefined },
  ])
  assert.deepEqual(launchCalls, [
    { provider: "codex", model: "codex/gpt-5.4", effort: "high", agentId: "agent-1" },
    { provider: "codex", model: "codex/gpt-5.4", effort: "high", agentId: "agent-2" },
  ])
  assert.equal(refreshCount, 4)
  assert.equal(flashedMessage, "spawned 2 agents from session defaults")
  assert.equal(currentSession.focused_agent_id, "agent-2")
})

test("agent spawn passes local directory as worktree and launches locally", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined }> = []
  const launchCalls: Array<{ agentId: string }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string) => {
      spawnCalls.push({ worktreeId, machineRef })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        state: "Focused",
      })
      const session = makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] })
      return { agent, session }
    },
    launchAgentProviderRun: async (_provider: string, _model: string, _variant: string, agentId: string) => {
      launchCalls.push({ agentId })
      return {
        id: "provider-run-1",
        session_id: "session-1",
        agent_instance_id: agentId,
        adapter_key: "opencode",
        provider: "opencode",
        account_profile: "default",
        model: "openai/gpt-5",
        variant: "medium",
        usage_tokens_total: null,
        state: "running",
      }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --dir .",
    args: ["spawn", "review", "openai/gpt-5", "--dir", "."],
  })

  assert.equal(spawnCalls.length, 1)
  assert.equal(spawnCalls[0]?.worktreeId, process.cwd())
  assert.equal(spawnCalls[0]?.machineRef, undefined)
  assert.deepEqual(launchCalls, [{ agentId: "agent-2" }])
  assert.match(flashedMessage, /^spawned agent agent-2 \(review\) · local · worktree /)
})

test("agent spawn creates a local git worktree placement before spawning", async () => {
  const preparedWorktree = "/tmp/arroba-feature-worktree"
  const prepareCalls: Array<{ targetDirectory?: string; branch?: string; fromRef?: string }> = []
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    prepareLocalGitWorktree: async (options: { targetDirectory?: string; branch?: string; fromRef?: string }) => {
      prepareCalls.push(options)
      return preparedWorktree
    },
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string) => {
      spawnCalls.push({ worktreeId, machineRef })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        state: "Focused",
      })
      return { agent, session: makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] }) }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --worktree ../feature --branch feature/test --from main",
    args: ["spawn", "review", "openai/gpt-5", "--worktree", "../feature", "--branch", "feature/test", "--from", "main"],
  })

  assert.equal(prepareCalls.length, 1)
  assert.equal(prepareCalls[0]?.targetDirectory, "../feature")
  assert.equal(prepareCalls[0]?.branch, "feature/test")
  assert.equal(prepareCalls[0]?.fromRef, "main")
  assert.deepEqual(spawnCalls, [{ worktreeId: preparedWorktree, machineRef: undefined }])
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · local · worktree /tmp/arroba-feature-worktree")
})

test("agent spawn with machine requires directory and does not launch local provider", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined; placement: unknown }> = []
  const kernelChecks: string[] = []
  let launchCount = 0
  let providerRunStateSet: RuntimeProviderRun | null | "unset" = "unset"
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    listRemoteMachineKernels: async (machineRef: string) => {
      kernelChecks.push(machineRef)
      return [{
        kernel_id: "kernel-worker",
        machine_id: "machine-worker",
        accepting_remote_leases: true,
        available_providers: ["opencode"],
      }]
    },
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string, placement?: unknown) => {
      spawnCalls.push({ worktreeId, machineRef, placement })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: worktreeId ?? null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
        state: "Focused",
      })
      const session = makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] })
      return { agent, session }
    },
    launchAgentProviderRun: async () => {
      launchCount += 1
      throw new Error("remote spawn should not launch local provider")
    },
    setProviderRunState: (run: RuntimeProviderRun | null) => { providerRunStateSet = run },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.deepEqual(kernelChecks, ["worker"])
  assert.deepEqual(spawnCalls, [{ worktreeId: "/srv/project", machineRef: "worker", placement: undefined }])
  assert.equal(launchCount, 0)
  assert.equal(providerRunStateSet, null)
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · remote machine-worker · worktree /srv/project")
})

test("agent spawn with machine can use the worker default directory", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (provider: string, alias?: string, model?: string, _effort?: string, worktreeId?: string, machineRef?: string) => {
      spawnCalls.push({ worktreeId, machineRef })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        provider,
        model: model ?? null,
        worktree_id: null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
        state: "Focused",
      })
      return { agent, session: makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] }) }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review --machine worker",
    args: ["spawn", "review", "--machine", "worker"],
  })

  assert.deepEqual(spawnCalls, [{ worktreeId: undefined, machineRef: "worker" }])
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · remote machine-worker")
})

test("agent spawn rejects machine and reusable slice together before provisioning", async () => {
  let spawnCount = 0
  let createSliceCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    createSlice: async () => {
      createSliceCount += 1
      throw new Error("should not create slice")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --slice slice-existing --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--slice", "slice-existing", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(createSliceCount, 0)
  assert.equal(flashedMessage, "usage: /agent spawn uses either --machine/--kernel or a reusable --slice, not both")
})

test("agent spawn with machine blocks workers that reject remote leases", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: false,
      available_providers: ["opencode"],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no kernel accepting remote agents; next: enable remote leases on kernel kernel-worker or choose another worker")
})

test("agent spawn with machine names workers without provider CLIs", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: [],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no accepting kernel with provider CLIs; next: configure provider CLIs on kernel kernel-worker or choose another worker")
})

test("agent spawn with machine blocks unknown worker readiness", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      available_providers: ["opencode"],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no kernel with known remote readiness; next: run /machine kernels worker, refresh relay inventory, or choose another worker")
})

test("agent spawn with machine blocks workers without the selected provider", async () => {
  let spawnCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    currentProviderId: () => "opencode",
    listRemoteMachineKernels: async () => [{
      kernel_id: "kernel-worker",
      machine_id: "machine-worker",
      accepting_remote_leases: true,
      available_providers: ["codex"],
    }],
    spawnAgent: async () => {
      spawnCount += 1
      throw new Error("should not spawn")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --dir /srv/project",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--dir", "/srv/project"],
  })

  assert.equal(spawnCount, 0)
  assert.equal(flashedMessage, "remote machine worker has no accepting kernel with provider opencode; next: choose a worker with opencode or change the agent provider")
})

test("agent spawn with machine forwards remote git worktree placement", async () => {
  const spawnCalls: Array<{ worktreeId: string | undefined; machineRef: string | undefined; placement: unknown }> = []
  let launchCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    spawnAgent: async (_provider: string, alias?: string, _model?: string, _effort?: string, worktreeId?: string, machineRef?: string, placement?: unknown) => {
      spawnCalls.push({ worktreeId, machineRef, placement })
      const agent = makeAgent({
        id: "agent-2",
        agent_ref: "agent-2",
        alias: alias ?? null,
        worktree_id: worktreeId ?? null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
      })
      return { agent, session: makeSession({ focused_agent_id: agent.id, agents: [makeAgent(), agent] }) }
    },
    launchAgentProviderRun: async () => {
      launchCount += 1
      throw new Error("remote spawn should not launch local provider")
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent spawn review openai/gpt-5 --machine worker --worktree /srv/project-feature --branch feature/remote --from main",
    args: ["spawn", "review", "openai/gpt-5", "--machine", "worker", "--worktree", "/srv/project-feature", "--branch", "feature/remote", "--from", "main"],
  })

  assert.deepEqual(spawnCalls, [{
    worktreeId: "/srv/project-feature",
    machineRef: "worker",
    placement: {
      target_directory: "/srv/project-feature",
      branch: "feature/remote",
      from_ref: "main",
    },
  }])
  assert.equal(launchCount, 0)
  assert.equal(flashedMessage, "spawned agent agent-2 (review) · remote machine-worker · worktree /srv/project-feature")
})

test("agent mode updates the focused agent through shared agent config", async () => {
  const baseAgent = makeAgent()
  const updatedAgent = makeAgent({ execution_mode_override: "plan" })
  const updatedSession = makeSession({ agents: [updatedAgent] })
  const updateCalls: Array<{ sessionId: string; agentId: string; executionMode: string | null | undefined; clearExecutionMode: boolean | undefined }> = []
  const appliedSessions: RuntimeSession[] = []
  let refreshCount = 0
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    resolveSessionAgent: () => ({ agent: baseAgent }),
    updateAgentConfig: async (sessionId: string, agentId: string, options: { executionMode?: "build" | "plan" | null; clearExecutionMode?: boolean }) => {
      updateCalls.push({
        sessionId,
        agentId,
        executionMode: options.executionMode,
        clearExecutionMode: options.clearExecutionMode,
      })
      return { agent: updatedAgent, session: updatedSession }
    },
    applySessionState: (session: RuntimeSession) => { appliedSessions.push(session) },
    refreshAgentPanes: async () => { refreshCount += 1 },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent mode plan",
    args: ["mode", "plan"],
  })

  assert.deepEqual(updateCalls, [{
    sessionId: "session-1",
    agentId: "agent-1",
    executionMode: "plan",
    clearExecutionMode: false,
  }])
  assert.deepEqual(appliedSessions, [updatedSession])
  assert.equal(refreshCount, 1)
  assert.equal(flashedMessage, "agent-1 mode: plan (agent)")
})

test("agent mode inherit clears the focused agent override", async () => {
  const overriddenAgent = makeAgent({ execution_mode_override: "plan" })
  const inheritedAgent = makeAgent({ execution_mode_override: null })
  const inheritedSession = makeSession({
    agents: [inheritedAgent],
    config_state: {
      version: 1,
      values: { "agents.mode": "build" },
      updated_by_attachment_id: "attachment-1",
    },
  })
  const updateCalls: Array<{ executionMode: string | null | undefined; clearExecutionMode: boolean | undefined }> = []
  let flashedMessage = ""
  const handlers = createCommandActionHandlers(makeCommandDeps({
    resolveSessionAgent: () => ({ agent: overriddenAgent }),
    updateAgentConfig: async (_sessionId: string, _agentId: string, options: { executionMode?: "build" | "plan" | null; clearExecutionMode?: boolean }) => {
      updateCalls.push({
        executionMode: options.executionMode,
        clearExecutionMode: options.clearExecutionMode,
      })
      return { agent: inheritedAgent, session: inheritedSession }
    },
    flashFooter: (message: string) => { flashedMessage = message },
  }))

  await handlers.handleAgentCommand({
    kind: "agent",
    raw: "/agent mode inherit",
    args: ["mode", "inherit"],
  })

  assert.deepEqual(updateCalls, [{ executionMode: null, clearExecutionMode: true }])
  assert.equal(flashedMessage, "agent-1 mode: build (session)")
})

test("cycle agent focus keeps split pane contents stable within the same screen", async () => {
  const agentA = makeAgent({ id: "agent-a", agent_ref: "agent-a" })
  const agentB = makeAgent({ id: "agent-b", agent_ref: "agent-b" })
  const agentC = makeAgent({ id: "agent-c", agent_ref: "agent-c" })
  let currentSession = makeSession({
    focused_agent_id: "agent-a",
    agents: [agentA, agentB, agentC],
  })
  let refreshCount = 0
  let flashedMessage = ""

  const handlers = createCommandActionHandlers({
    workspace: "workspace-1",
    worktree: "worktree-1",
    accountProfile: "default",
    isAttached: () => true,
    sessionState: () => currentSession,
    attachmentState: (): RuntimeAttachment => ({ id: "attachment-1", session_id: "session-1" }),
    providerRunState: (): RuntimeProviderRun | null => ({
      id: "provider-run-1",
      session_id: "session-1",
      agent_instance_id: "agent-a",
      adapter_key: "opencode",
      provider: "opencode",
      account_profile: "default",
      model: "openai/gpt-5",
      variant: "medium",
      usage_tokens_total: null,
      state: "running",
    }),
    currentModelId: () => "openai/gpt-5",
    currentVariantId: () => "medium",
    currentProviderId: () => "opencode",
    focusedAgentId: () => currentSession.focused_agent_id,
    multiAgentResponseLayout: () => "split",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message) => { flashedMessage = message },
    appendNotice: () => {},
    formatError: (error) => String(error),
    createSession: async () => ({ id: "session-1", alias: null }),
    attachBinding: async () => {},
    resolveSession: async () => ({ id: "session-1", alias: null }),
    listSessions: async () => [],
    deleteSessionByRef: async () => ({ id: "session-1", alias: null }),
    transitionToNoSession: () => {},
    applyModelSelection: async () => {},
    applyVariantSelection: async () => {},
    setMultiAgentResponseLayout: () => {},
    applyResponseLayout: () => {},
    updateSessionResponseLayout: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    updateSessionConfig: async () => ({
      session: currentSession,
      config: currentSession.config_state,
    }),
    applySessionState: (session) => {
      currentSession = session
    },
    refreshAgentPanes: async () => {
      refreshCount += 1
    },
    saveUiPreferences: async () => {},
    rebuildTranscript: () => {},
    requestRender: () => {},
    cycleAgentFocus: async () => ({
      agent: agentB,
      session: {
        ...currentSession,
        active_provider_run_id: "provider-run-1",
        focused_agent_id: "agent-b",
      },
    }),
    launchAgentProviderRun: async () => {
      throw new Error("should not launch a new run")
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async () => ({ agent: agentB, session: currentSession }),
    destroyAgent: async () => currentSession,
    focusAgent: async () => ({ agent: agentB, session: currentSession }),
    resolveSessionAgent: () => ({ agent: currentSession.agents[0] ?? null }),
    workflowScreenActive: () => false,
    showWorkflowScreen: () => {},
    selectWorkflowCanvas: () => {},
    replaceWorkflowDefinitions: () => {},
    upsertWorkflowDefinition: () => {},
    createWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    listWorkflows: async () => [],
    resolveWorkflow: async () => ({ workflow: { id: "workflow-1", alias: null } }),
    assignWorkflowAlias: async () => null,
    createWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    assignWorkflowEndpointAlias: async () => ({ endpoint: { id: "endpoint-1", alias: "entry", entry_node_id: "node-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    bindWorkflowEndpoint: async () => ({ endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowNode: async () => ({ node: { id: "node-1", agent_id: "agent-1" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    addWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    removeWorkflowEdge: async () => ({ edge: { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }, workflow: { id: "workflow-1", alias: null }, session: makeSession() }),
    formatAgentLabel: (agent) => agent?.agent_ref ?? "",
    refreshSplitPaneFocusRepaint: () => {},
    formatSessionList: () => "",
  })

  await handlers.handleCycleAgentFocus()

  assert.equal(refreshCount, 0)
  assert.equal(flashedMessage, "cycled to agent agent-b")
  assert.equal(currentSession.focused_agent_id, "agent-b")
})
