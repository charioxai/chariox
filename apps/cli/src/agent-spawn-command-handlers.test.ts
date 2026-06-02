import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./cli-types.js"
import { handleAgentSpawnCommand } from "./agent-spawn-command-handlers.js"

test("agent spawn command count inherits session defaults and launches each agent", async () => {
  let currentSession = session()
  const spawnedAgentIds: string[] = []
  const launchAgentIds: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => String(error),
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    launchAgentProviderRun: async (_provider, _model, _variant, agentId) => {
      launchAgentIds.push(agentId)
      return providerRun({ agent_instance_id: agentId })
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async () => {
      const id = `agent-${spawnedAgentIds.length + 1}`
      const nextAgent = agent({ id, agent_ref: id, provider: "codex", model: "codex/gpt-5.4", effort: "high" })
      spawnedAgentIds.push(id)
      currentSession = session({ focused_agent_id: id, agents: [...currentSession.agents, nextAgent] })
      return { agent: nextAgent, session: currentSession }
    },
    refreshSplitPaneFocusRepaint: () => {},
  }, ["2"])

  assert.deepEqual(spawnedAgentIds, ["agent-1", "agent-2"])
  assert.deepEqual(launchAgentIds, ["agent-1", "agent-2"])
  assert.equal(flashedMessage, "spawned 2 agents from session defaults")
})

test("agent spawn command can create and start a new slice", async () => {
  let currentSession = session()
  const calls: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    prepareLocalGitWorktree: async (options) => {
      calls.push(`prepare:${options.targetDirectory}:${options.branch}`)
      return options.targetDirectory ?? "/workspace-feature"
    },
    createSlice: async (options) => {
      calls.push(`create:${options.displayMode}:${options.worktreeId}`)
      return slice({ id: "slice-created", display_mode: options.displayMode, worktree_id: options.worktreeId })
    },
    startSlice: async (sliceRef) => {
      calls.push(`start:${sliceRef}`)
      return slice({ id: sliceRef, display_mode: "headed" })
    },
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => { calls.push("rebuild") },
    launchAgentProviderRun: async () => {
      throw new Error("remote slice spawn should not launch locally")
    },
    setProviderRunState: (run) => { calls.push(`run:${run ? run.id : "null"}`) },
    refreshSessionState: async () => currentSession,
    spawnAgent: async (_provider, alias, _model, _effort, worktreeId, _machineRef, _worktreePlacement, sliceRef) => {
      calls.push(`spawn:${alias}:${worktreeId}:${sliceRef}`)
      const nextAgent = agent({
        id: "agent-slice",
        agent_ref: "agent-slice",
        alias: alias ?? null,
        remote_execution: {
          worker_kernel_id: "kernel-slice",
          worker_machine_id: "machine-slice",
          execution_lease_id: "lease-slice",
          leased_agent_id: "worker-agent",
        },
      })
      currentSession = session({ focused_agent_id: nextAgent.id, agents: [...currentSession.agents, nextAgent] })
      return { agent: nextAgent, session: currentSession }
    },
    refreshSplitPaneFocusRepaint: () => { calls.push("repaint") },
  }, ["builder", "codex/gpt-5.4", "--slice", "new", "--worktree", "/workspace-feature", "--branch", "feature/login"])

  assert.deepEqual(calls, [
    "prepare:/workspace-feature:feature/login",
    "create:headless:/workspace-feature",
    "start:slice-created",
    "spawn:builder:/workspace-feature:slice-created",
    "run:null",
    "rebuild",
    "repaint",
  ])
  assert.equal(flashedMessage, "spawned agent agent-slice (builder) · slice slice-created · worktree /workspace-feature · worker machine-slice")
})

test("agent spawn command can create a headed slice with separate display option", async () => {
  let currentSession = session()
  const calls: string[] = []

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: () => {},
    formatError: (error) => error instanceof Error ? error.message : String(error),
    createSlice: async (options) => {
      calls.push(`create:${options.displayMode}:${options.worktreeId}`)
      return slice({ id: "slice-created", display_mode: options.displayMode, worktree_id: options.worktreeId })
    },
    startSlice: async (sliceRef) => {
      calls.push(`start:${sliceRef}`)
      return slice({ id: sliceRef, display_mode: "headed" })
    },
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    launchAgentProviderRun: async () => {
      throw new Error("remote slice spawn should not launch locally")
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async (_provider, _alias, _model, _effort, worktreeId, _machineRef, _worktreePlacement, sliceRef) => {
      calls.push(`spawn:${worktreeId}:${sliceRef}`)
      const nextAgent = agent({
        id: "agent-slice",
        agent_ref: "agent-slice",
        remote_execution: {
          worker_kernel_id: "kernel-slice",
          worker_machine_id: "machine-slice",
          execution_lease_id: "lease-slice",
          leased_agent_id: "worker-agent",
        },
      })
      currentSession = session({ focused_agent_id: nextAgent.id, agents: [...currentSession.agents, nextAgent] })
      return { agent: nextAgent, session: currentSession }
    },
    refreshSplitPaneFocusRepaint: () => {},
  }, ["builder", "codex/gpt-5.4", "--slice", "new", "--slice-display", "headed"])

  assert.deepEqual(calls, [
    "create:headed:/workspace",
    "start:slice-created",
    "spawn:undefined:slice-created",
  ])
})

test("agent spawn command rejects slice display without a new slice", async () => {
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    createSlice: async () => {
      throw new Error("slice display without new should not create a slice")
    },
    startSlice: async () => {
      throw new Error("slice display without new should not start a slice")
    },
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    launchAgentProviderRun: async () => {
      throw new Error("slice display without new should not launch")
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => session(),
    spawnAgent: async () => {
      throw new Error("slice display without new should not spawn")
    },
    refreshSplitPaneFocusRepaint: () => {},
  }, ["builder", "codex/gpt-5.4", "--slice", "slice-existing", "--slice-display", "headed"])

  assert.equal(flashedMessage, "usage: /agent spawn --slice-display requires --slice new")
})

test("agent spawn command treats --slice off as normal local placement", async () => {
  let currentSession = session()
  const calls: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    prepareLocalGitWorktree: async (options) => {
      calls.push(`prepare:${options.targetDirectory}:${options.branch}`)
      return options.targetDirectory ?? "/workspace-feature"
    },
    createSlice: async () => {
      throw new Error("slice off should not create a slice")
    },
    startSlice: async () => {
      throw new Error("slice off should not start a slice")
    },
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => { calls.push("rebuild") },
    launchAgentProviderRun: async (_provider, _model, _variant, agentId) => {
      calls.push(`launch:${agentId}`)
      return providerRun({ agent_instance_id: agentId })
    },
    setProviderRunState: (run) => { calls.push(`run:${run ? run.id : "null"}`) },
    refreshSessionState: async () => currentSession,
    spawnAgent: async (_provider, alias, _model, _effort, worktreeId, _machineRef, _worktreePlacement, sliceRef) => {
      calls.push(`spawn:${alias}:${worktreeId}:${sliceRef ?? "none"}`)
      const nextAgent = agent({
        id: "agent-local",
        agent_ref: "agent-local",
        alias: alias ?? null,
      })
      currentSession = session({ focused_agent_id: nextAgent.id, agents: [...currentSession.agents, nextAgent] })
      return { agent: nextAgent, session: currentSession }
    },
    refreshSplitPaneFocusRepaint: () => { calls.push("repaint") },
  }, ["builder", "codex/gpt-5.4", "--slice", "off", "--worktree", "/workspace-feature", "--branch", "feature/login"])

  assert.deepEqual(calls, [
    "prepare:/workspace-feature:feature/login",
    "spawn:builder:/workspace-feature:none",
    "launch:agent-local",
    "run:run-1",
    "rebuild",
    "repaint",
  ])
  assert.equal(flashedMessage, "spawned agent agent-local (builder) · local · worktree /workspace-feature")
})

test("agent spawn command targets an exact kernel without machine resolution", async () => {
  let currentSession = session()
  const calls: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    listRemoteMachineKernels: async () => {
      throw new Error("--kernel should not list machine kernels")
    },
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => { calls.push("rebuild") },
    launchAgentProviderRun: async () => {
      throw new Error("kernel-targeted spawn should not launch locally")
    },
    setProviderRunState: (run) => { calls.push(`run:${run ? run.id : "null"}`) },
    refreshSessionState: async () => currentSession,
    spawnAgent: async (_provider, alias, _model, _effort, worktreeId, kernelRef, placement, sliceRef) => {
      calls.push(`spawn:${alias}:${worktreeId ?? "default"}:${kernelRef}:${placement ? "placement" : "none"}:${sliceRef ?? "none"}`)
      const nextAgent = agent({
        id: "agent-worker",
        agent_ref: "agent-worker",
        alias: alias ?? null,
        worktree_id: null,
        remote_execution: {
          worker_kernel_id: "kernel-worker",
          worker_machine_id: "machine-worker",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-agent-1",
        },
      })
      currentSession = session({ focused_agent_id: nextAgent.id, agents: [...currentSession.agents, nextAgent] })
      return { agent: nextAgent, session: currentSession }
    },
    refreshSplitPaneFocusRepaint: () => { calls.push("repaint") },
  }, ["builder", "codex/gpt-5.4", "--kernel", "kernel-worker"])

  assert.deepEqual(calls, [
    "spawn:builder:default:kernel-worker:none:none",
    "run:null",
    "rebuild",
    "repaint",
  ])
  assert.equal(flashedMessage, "spawned agent agent-worker (builder) · remote machine-worker")
})

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-0",
    agent_ref: "agent-0",
    session_id: "session-1",
    alias: null,
    provider: "opencode",
    model: "gpt-5.4",
    effort: "medium",
    worktree_id: "worktree-1",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "slice-1",
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker",
    os: "linux",
    status: "running",
    workspace_mount: null,
    worker_kernel_ref: "slice:slice-1",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    providers: ["codex"],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 0,
    status: "Running",
    active_provider_run_id: null,
    attachment_ids: ["attachment-1"],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-0",
    max_agents: 6,
    agents: [agent()],
    workflows: [],
    workflow_runs: [],
    config_state: {
      version: 0,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function providerRun(overrides: Partial<RuntimeProviderRun> = {}): RuntimeProviderRun {
  return {
    id: "run-1",
    session_id: "session-1",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    provider: "codex",
    account_profile: "default",
    model: "codex/gpt-5.4",
    variant: "high",
    usage_tokens_total: null,
    state: "Running",
    started_at_ms: 0,
    ...overrides,
  }
}
