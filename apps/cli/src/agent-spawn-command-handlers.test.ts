import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./cli-types.js"
import { handleAgentSpawnCommand, type AgentSpawnCommandHandlerDeps } from "./agent-spawn-command-handlers.js"

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

test("agent spawn command imports an external provider session as a new agent", async () => {
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
    applySessionState: (nextSession) => {
      calls.push(`apply:${nextSession.id}`)
      currentSession = nextSession
    },
    refreshAgentPanes: async (nextSession) => { calls.push(`panes:${nextSession.id}`) },
    rebuildTranscript: () => { calls.push("rebuild") },
    launchAgentProviderRun: async () => {
      throw new Error("external import should launch through the kernel import response")
    },
    setProviderRunState: (run) => { calls.push(`run:${run?.id ?? "none"}`) },
    refreshSessionState: async () => currentSession,
    spawnAgent: async () => {
      throw new Error("external import should not call normal spawn")
    },
    importExternalProviderAgent: async (externalSessionId) => {
      calls.push(`import:${externalSessionId}`)
      const importedAgent = agent({ id: "agent-imported", agent_ref: "provider-thread" })
      currentSession = session({ focused_agent_id: importedAgent.id, agents: [...currentSession.agents, importedAgent] })
      return {
        session: currentSession,
        agent: importedAgent,
        providerRun: providerRun({ id: "run-imported", agent_instance_id: importedAgent.id }),
      }
    },
    refreshSplitPaneFocusRepaint: () => { calls.push("repaint") },
  }, ["--external", "codex:thread-1"])

  assert.deepEqual(calls, [
    "import:codex:thread-1",
    "apply:session-1",
    "panes:session-1",
    "run:run-imported",
    "rebuild",
    "repaint",
  ])
  assert.equal(flashedMessage, "imported external session codex:thread-1 as provider-thread")
})

test("agent spawn command rejects external imports with placement options", async () => {
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: (message) => { flashedMessage = message },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    applySessionState: () => {},
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    launchAgentProviderRun: async () => providerRun(),
    setProviderRunState: () => {},
    refreshSessionState: async () => session(),
    spawnAgent: async () => {
      throw new Error("invalid external import should not spawn")
    },
    refreshSplitPaneFocusRepaint: () => {},
  }, ["--external", "codex:thread-1", "--slice", "new"])

  assert.equal(flashedMessage, "usage: /agent spawn --external <external-session-id> does not accept placement options")
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

test("agent spawn command creates a new slice on an exact kernel without also spawning on the kernel", async () => {
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
      calls.push(`create:${options.workerKernelRef}:${options.worktreeId}`)
      return slice({ id: "slice-created", display_mode: options.displayMode, worktree_id: options.worktreeId })
    },
    startSlice: async (sliceRef) => {
      calls.push(`start:${sliceRef}`)
      return slice({ id: sliceRef, display_mode: "headless" })
    },
    applySessionState: (nextSession) => { currentSession = nextSession },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => {},
    launchAgentProviderRun: async () => {
      throw new Error("slice spawn should not launch locally")
    },
    setProviderRunState: () => {},
    refreshSessionState: async () => currentSession,
    spawnAgent: async (_provider, _alias, _model, _effort, _worktreeId, machineRef, _worktreePlacement, sliceRef) => {
      calls.push(`spawn:${machineRef ?? "none"}:${sliceRef}`)
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
  }, ["builder", "codex/gpt-5.4", "--kernel", "kernel-worker", "--slice", "new"])

  assert.deepEqual(calls, [
    "create:kernel-worker:/workspace",
    "start:slice-created",
    "spawn:none:slice-created",
  ])
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

test("agent spawn command reuses resolved slice id after validating scope", async () => {
  let currentSession = session()
  const calls: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    ...baseSpawnDeps({
      currentSession: () => currentSession,
      setSession: (nextSession) => { currentSession = nextSession },
      calls,
      flash: (message) => { flashedMessage = message },
    }),
    startSlice: async (sliceRef) => {
      calls.push(`start:${sliceRef}`)
      return slice({
        id: "slice-resolved",
        name: "dev",
        workspace_id: "/workspace",
        worktree_id: "/workspace",
      })
    },
    spawnAgent: async (_provider, alias, _model, _effort, worktreeId, machineRef, _worktreePlacement, sliceRef) => {
      calls.push(`spawn:${alias}:${worktreeId ?? "default"}:${machineRef ?? "none"}:${sliceRef}`)
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
  }, ["builder", "codex/gpt-5.4", "--slice", "dev"])

  assert.deepEqual(calls, [
    "start:dev",
    "spawn:builder:default:none:slice-resolved",
    "run:null",
    "rebuild",
    "repaint",
  ])
  assert.equal(flashedMessage, "spawned agent agent-slice (builder) · slice slice-resolved · worktree worktree-1 · worker machine-slice")
})

test("agent spawn command rejects reusable slices from another worktree", async () => {
  const calls: string[] = []
  let flashedMessage = ""
  let flashedTone = ""

  await handleAgentSpawnCommand({
    ...baseSpawnDeps({
      currentSession: () => session(),
      calls,
      flash: (message, tone) => {
        flashedMessage = message
        flashedTone = tone
      },
    }),
    startSlice: async (sliceRef) => {
      calls.push(`start:${sliceRef}`)
      return slice({
        id: "slice-other",
        name: "dev",
        workspace_id: "/workspace",
        worktree_id: "/workspace/other",
      })
    },
    spawnAgent: async () => {
      throw new Error("stale reusable slice should not spawn an agent")
    },
  }, ["builder", "codex/gpt-5.4", "--slice", "dev"])

  assert.deepEqual(calls, ["start:dev"])
  assert.equal(flashedTone, "error")
  assert.equal(flashedMessage, "slice dev is scoped to worktree /workspace/other, not /workspace; choose a slice for this worktree, use --slice new, or use --slice off")
})

test("agent spawn command rejects reusable slices that did not reach running", async () => {
  const calls: string[] = []
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    ...baseSpawnDeps({
      currentSession: () => session(),
      calls,
      flash: (message) => { flashedMessage = message },
    }),
    startSlice: async (sliceRef) => {
      calls.push(`start:${sliceRef}`)
      return slice({
        id: "slice-starting",
        name: "dev",
        workspace_id: "/workspace",
        worktree_id: "/workspace",
        status: "starting",
      })
    },
    spawnAgent: async () => {
      throw new Error("non-running reusable slice should not spawn an agent")
    },
  }, ["builder", "codex/gpt-5.4", "--slice", "dev"])

  assert.deepEqual(calls, ["start:dev"])
  assert.equal(flashedMessage, "slice dev is starting; next: run /slice status dev, /slice logs dev, then retry after it is running")
})

test("agent spawn command fails loudly when reusable slice lifecycle is unavailable", async () => {
  let flashedMessage = ""

  await handleAgentSpawnCommand({
    ...baseSpawnDeps({
      currentSession: () => session(),
      flash: (message) => { flashedMessage = message },
    }),
    spawnAgent: async () => {
      throw new Error("unchecked reusable slice should not spawn an agent")
    },
  }, ["builder", "codex/gpt-5.4", "--slice", "dev"])

  assert.equal(flashedMessage, "slice reuse is unavailable in this build")
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

function baseSpawnDeps(options: {
  currentSession: () => RuntimeSession
  setSession?: (session: RuntimeSession) => void
  calls?: string[]
  flash?: (message: string, tone: "info" | "error") => void
}): AgentSpawnCommandHandlerDeps {
  return {
    currentWorkspaceTarget: () => "/workspace",
    currentWorktreeTarget: () => "/workspace",
    currentModelId: () => "codex/gpt-5.4",
    currentVariantId: () => "high",
    currentProviderId: () => "codex",
    flashFooter: options.flash ?? (() => {}),
    formatError: (error) => error instanceof Error ? error.message : String(error),
    applySessionState: (nextSession) => {
      options.setSession?.(nextSession)
    },
    refreshAgentPanes: async () => {},
    rebuildTranscript: () => { options.calls?.push("rebuild") },
    launchAgentProviderRun: async () => {
      throw new Error("base spawn deps should not launch locally")
    },
    setProviderRunState: (run) => { options.calls?.push(`run:${run ? run.id : "null"}`) },
    refreshSessionState: async () => options.currentSession(),
    spawnAgent: async () => {
      throw new Error("base spawn deps should override spawnAgent")
    },
    refreshSplitPaneFocusRepaint: () => { options.calls?.push("repaint") },
  }
}

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
