import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
  WaitingRoomRemoteKernelView,
} from "./cli-types.js"
import {
  parseAgentSpawnOptions,
  resolveLocalPlacement,
  type LocalGitWorktreeOptions,
  type RemoteGitWorktreePlacement,
} from "./command-worktree-placement.js"

type FooterTone = "info" | "error"

type AgentSpawnPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

export type AgentSpawnCommandHandlerDeps = {
  currentWorkspaceTarget: () => string
  currentWorktreeTarget: () => string
  currentModelId: () => string
  currentVariantId: () => string
  currentProviderId: () => string
  flashFooter: (message: string, tone: FooterTone) => void
  formatError: (error: unknown) => string
  prepareLocalGitWorktree?: (options: LocalGitWorktreeOptions) => Promise<string>
  createSlice?: (options: {
    name: string
    displayMode: "headless" | "headed"
    workspaceId: string
    worktreeId: string
    workspaceMount: string
  }) => Promise<SliceRecord>
  startSlice?: (sliceRef: string) => Promise<SliceRecord>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  rebuildTranscript: () => void
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  spawnAgent: (
    provider?: string | null,
    alias?: string,
    model?: string | null,
    effort?: string | null,
    worktreeId?: string,
    machineRef?: string,
    worktreePlacement?: RemoteGitWorktreePlacement | undefined,
    sliceRef?: string,
  ) => Promise<AgentSpawnPayload>
  listRemoteMachineKernels?: (machineRef: string) => Promise<WaitingRoomRemoteKernelView[]>
  refreshSplitPaneFocusRepaint: () => void
}

export async function handleAgentSpawnCommand(
  deps: AgentSpawnCommandHandlerDeps,
  spawnArgs: string[],
): Promise<void> {
  try {
    const parsed = parseAgentSpawnOptions(spawnArgs)
    if (parsed.error) {
      deps.flashFooter(parsed.error, "error")
      return
    }
    const count = parseSpawnCount(parsed.positional[0])
    if (
      count !== null
      && (parsed.positional.length > 1 || parsed.directory || parsed.machineRef || parsed.sliceRef || parsed.sliceDisplayMode || parsed.gitWorktree || parsed.branch || parsed.fromRef)
    ) {
      deps.flashFooter("usage: /agent spawn <count>", "error")
      return
    }
    if (count !== null && parsed.positional.length === 1) {
      for (let index = 0; index < count; index += 1) {
        await spawnAndLaunchAgent(deps, {})
      }
      deps.flashFooter(
        `spawned ${count} agent${count === 1 ? "" : "s"} from session defaults`,
        "info",
      )
      return
    }

    const alias = parsed.positional[0]
    const model = parsed.positional[1]
    const provider = model ? deps.currentProviderId() : null
    const effectiveProvider = provider ?? deps.currentProviderId()
    const effort = model ? deps.currentVariantId() : null
    const remoteGitPlacement = parsed.machineRef && (parsed.gitWorktree || parsed.branch || parsed.fromRef)
      ? {
          target_directory: parsed.gitWorktree ?? null,
          branch: parsed.branch ?? null,
          from_ref: parsed.fromRef ?? null,
        }
      : undefined
    if (parsed.machineRef) {
      await validateRemoteMachineSpawnTarget(deps, parsed.machineRef, effectiveProvider)
    }
    const worktreeId = await resolveLocalPlacement({
      directory: parsed.directory,
      gitWorktree: parsed.gitWorktree,
      branch: parsed.branch,
      fromRef: parsed.fromRef,
      machineRef: parsed.machineRef,
      label: "agent working directory",
    }, {
      baseDirectory: deps.currentWorktreeTarget(),
      prepareLocalGitWorktree: deps.prepareLocalGitWorktree,
    })
    const sliceRef = await prepareSliceForSpawn(deps, parsed.sliceRef, parsed.sliceDisplayMode, worktreeId)
    const payload = await spawnAndLaunchAgent(deps, {
      provider,
      alias,
      model: model ?? null,
      effort,
      worktreeId,
      machineRef: parsed.machineRef,
      worktreePlacement: remoteGitPlacement,
      sliceRef,
    })
    deps.flashFooter(formatSpawnedAgentFooter(payload.agent, {
      requestedAlias: alias,
      requestedMachineRef: parsed.machineRef,
      resolvedWorktreeId: worktreeId,
      sliceRef,
    }), "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

async function validateRemoteMachineSpawnTarget(
  deps: AgentSpawnCommandHandlerDeps,
  machineRef: string,
  provider: string,
): Promise<void> {
  if (!deps.listRemoteMachineKernels) {
    return
  }
  const kernels = await deps.listRemoteMachineKernels(machineRef)
  if (kernels.length === 0) {
    throw new Error(`remote machine ${machineRef} has no live worker kernels; next: run /machine kernels ${machineRef} or choose another worker`)
  }
  const accepting = kernels.filter((kernel) => kernel.accepting_remote_leases !== false)
  if (accepting.length === 0) {
    throw new Error(`remote machine ${machineRef} has no kernel accepting remote agents; next: enable remote leases on a kernel or choose another worker`)
  }
  if (accepting.every((kernel) => (kernel.available_providers ?? []).length === 0)) {
    throw new Error(`remote machine ${machineRef} has no accepting kernel with provider CLIs; next: configure provider CLIs on the worker or choose another worker`)
  }
  if (!accepting.some((kernel) => (kernel.available_providers ?? []).includes(provider))) {
    throw new Error(`remote machine ${machineRef} has no accepting kernel with provider ${provider}; next: choose a worker with ${provider} or change the agent provider`)
  }
}

async function prepareSliceForSpawn(
  deps: AgentSpawnCommandHandlerDeps,
  sliceSelection: string | undefined,
  sliceDisplayMode: "headless" | "headed" | undefined,
  worktreeId: string | undefined,
): Promise<string | undefined> {
  if (!sliceSelection || sliceSelection === "off") {
    return undefined
  }
  const createMode = sliceCreateMode(sliceSelection, sliceDisplayMode)
  if (!createMode) {
    if (deps.startSlice) {
      await deps.startSlice(sliceSelection)
    }
    return sliceSelection
  }
  if (!deps.createSlice || !deps.startSlice) {
    throw new Error("slice creation is unavailable in this build")
  }
  const resolvedWorktree = worktreeId || deps.currentWorktreeTarget()
  const slice = await deps.createSlice({
    name: defaultSliceName(resolvedWorktree),
    displayMode: createMode.displayMode,
    workspaceId: deps.currentWorkspaceTarget(),
    worktreeId: resolvedWorktree,
    workspaceMount: resolvedWorktree,
  })
  const started = await deps.startSlice(slice.id)
  return started.id
}

function sliceCreateMode(value: string, displayMode: "headless" | "headed" | undefined): { displayMode: "headless" | "headed" } | null {
  if (value === "new") {
    return { displayMode: displayMode ?? "headless" }
  }
  return null
}

function defaultSliceName(worktreePath: string): string {
  const leaf = worktreePath.split("/").filter(Boolean).pop() || "workspace"
  const suffix = Date.now().toString(36).slice(-5)
  return `${leaf}-slice-${suffix}`.replace(/[^a-zA-Z0-9_.-]/g, "-")
}

function parseSpawnCount(value: string | undefined): number | null {
  if (!value || !/^\d+$/.test(value)) {
    return null
  }
  const count = Number(value)
  return Number.isInteger(count) && count > 0 ? count : null
}

function formatSpawnedAgentFooter(
  agent: AgentInstance,
  options: {
    requestedAlias?: string | undefined
    requestedMachineRef?: string | undefined
    resolvedWorktreeId?: string | undefined
    sliceRef?: string | undefined
  },
): string {
  const alias = agent.alias ?? options.requestedAlias ?? null
  const remote = agent.remote_execution ?? null
  const worker = remote?.worker_machine_id || remote?.worker_kernel_id || options.requestedMachineRef || null
  const placement = options.sliceRef
    ? `slice ${options.sliceRef}`
    : remote
      ? `remote ${worker ?? "worker"}`
      : "local"
  const worktree = options.resolvedWorktreeId || agent.worktree_id || null
  const parts = [
    `spawned agent ${agent.agent_ref}${alias ? ` (${alias})` : ""}`,
    placement,
    worktree ? `worktree ${worktree}` : null,
    options.sliceRef && worker ? `worker ${worker}` : null,
  ].filter(Boolean)
  return parts.join(" · ")
}

async function spawnAndLaunchAgent(
  deps: AgentSpawnCommandHandlerDeps,
  options: {
    provider?: string | null
    alias?: string | undefined
    model?: string | null
    effort?: string | null
    worktreeId?: string | undefined
    machineRef?: string | undefined
    worktreePlacement?: RemoteGitWorktreePlacement | undefined
    sliceRef?: string | undefined
  },
): Promise<AgentSpawnPayload> {
  const payload = await deps.spawnAgent(
    options.provider,
    options.alias,
    options.model,
    options.effort,
    options.worktreeId,
    options.machineRef,
    options.worktreePlacement,
    options.sliceRef,
  )
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  if (options.machineRef || options.sliceRef || payload.agent.remote_execution) {
    deps.setProviderRunState(null)
    const refreshedSession = await deps.refreshSessionState(payload.session.id)
    deps.applySessionState(refreshedSession)
    await deps.refreshAgentPanes(refreshedSession)
    deps.rebuildTranscript()
    deps.refreshSplitPaneFocusRepaint()
    return {
      agent: payload.agent,
      session: refreshedSession,
    }
  }
  const launchProvider = payload.agent.provider || options.provider || deps.currentProviderId()
  const launchModel = payload.agent.model || options.model || deps.currentModelId()
  const launchEffort = payload.agent.effort || options.effort || deps.currentVariantId()
  const run = await deps.launchAgentProviderRun(
    launchProvider,
    launchModel,
    launchEffort,
    payload.agent.id,
  )
  deps.setProviderRunState(run)
  const refreshedSession = await deps.refreshSessionState(payload.session.id)
  deps.applySessionState(refreshedSession)
  await deps.refreshAgentPanes(refreshedSession)
  deps.rebuildTranscript()
  deps.refreshSplitPaneFocusRepaint()
  return {
    agent: payload.agent,
    session: refreshedSession,
  }
}
