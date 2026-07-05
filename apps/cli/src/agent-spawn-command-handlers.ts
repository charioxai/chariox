import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
  WaitingRoomRemoteKernelView,
} from "./cli-types.js"
import {
  gitWorktreePlacementFromParse,
  parseAgentSpawnOptions,
  resolveLocalPlacement,
  type RemoteGitWorktreePlacement,
} from "./command-worktree-placement.js"
import { remoteKernelReadiness } from "@arroba/kernel-client/shell-remote-format"

type FooterTone = "info" | "error"

type AgentSpawnPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type AgentSpawnBatchPayload = {
  agents: AgentInstance[]
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
  createSlice?: (options: {
    name: string
    displayMode: "headless" | "headed"
    workspaceId: string
    worktreeId: string
    workspaceMount: string
    workerKernelRef?: string | null
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
  spawnAgents?: (
    agents: Array<{
      provider?: string | null | undefined
      alias?: string | undefined
      model?: string | null | undefined
      effort?: string | null | undefined
      worktreeId?: string | undefined
      machineRef?: string | undefined
      worktreePlacement?: RemoteGitWorktreePlacement | undefined
      sliceRef?: string | undefined
    }>,
  ) => Promise<AgentSpawnBatchPayload>
  launchAgentProviderRuns?: (
    provider: string,
    model: string,
    variant: string,
    agentIds: readonly string[],
  ) => Promise<{ runs: RuntimeProviderRun[]; failures: Array<{ index: number; agent_id?: string | null; message: string }> }>
  importExternalProviderAgent?: (
    externalSessionId: string,
  ) => Promise<AgentSpawnPayload & { providerRun?: RuntimeProviderRun | null }>
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
      && (parsed.positional.length > 1 || parsed.directory || parsed.machineRef || parsed.kernelRef || parsed.sliceRef || parsed.sliceDisplayMode || parsed.gitWorktree || parsed.branch || parsed.fromRef || parsed.externalSessionId)
    ) {
      deps.flashFooter("usage: /agent spawn <count>", "error")
      return
    }
    if (parsed.externalSessionId) {
      if (!deps.importExternalProviderAgent) {
        deps.flashFooter("unattached agent import is unavailable", "error")
        return
      }
      const payload = await deps.importExternalProviderAgent(parsed.externalSessionId)
      deps.applySessionState(payload.session)
      await deps.refreshAgentPanes(payload.session)
      deps.setProviderRunState(payload.providerRun ?? null)
      deps.rebuildTranscript()
      deps.refreshSplitPaneFocusRepaint()
      deps.flashFooter(`attached agent ${parsed.externalSessionId} as ${payload.agent.agent_ref}`, "info")
      return
    }
    if (count !== null && parsed.positional.length === 1) {
      if (!deps.spawnAgents || !deps.launchAgentProviderRuns) {
        for (let index = 0; index < count; index += 1) {
          await spawnAndLaunchAgent(deps, {})
        }
        deps.flashFooter(
          `spawned ${count} agent${count === 1 ? "" : "s"} from session defaults`,
          "info",
        )
        return
      }
      const payload = await deps.spawnAgents(Array.from({ length: count }, () => ({})))
      deps.applySessionState(payload.session)
      await deps.refreshAgentPanes(payload.session)
      const provider = deps.currentProviderId()
      const model = deps.currentModelId()
      const variant = deps.currentVariantId()
      const launched = await deps.launchAgentProviderRuns(
        provider,
        model,
        variant,
        payload.agents.map((agent) => agent.id),
      )
      deps.setProviderRunState(launched.runs.at(-1) ?? null)
      const refreshedSession = await deps.refreshSessionState(payload.session.id)
      deps.applySessionState(refreshedSession)
      await deps.refreshAgentPanes(refreshedSession)
      deps.rebuildTranscript()
      deps.refreshSplitPaneFocusRepaint()
      const failureSummary = launched.failures.length
        ? `; ${launched.failures.length} launch failure${launched.failures.length === 1 ? "" : "s"}`
        : ""
      deps.flashFooter(
        `spawned ${payload.agents.length} agent${payload.agents.length === 1 ? "" : "s"} from session defaults${failureSummary}`,
        launched.failures.length ? "error" : "info",
      )
      if (launched.failures.length) {
        return
      }
      return
    }

    const alias = parsed.positional[0]
    const model = parsed.positional[1]
    const provider = model ? deps.currentProviderId() : null
    const effectiveProvider = provider ?? deps.currentProviderId()
    const effort = model ? deps.currentVariantId() : null
    if (parsed.metaagent) {
      throw new Error("creating separate metaagents is deprecated; send /meta <task> to a regular agent to enter meta mode")
    }
    const remoteRef = parsed.kernelRef ?? parsed.machineRef
    const worktreePlacement = gitWorktreePlacementFromParse(parsed)
    if (parsed.machineRef) {
      await validateRemoteMachineSpawnTarget(deps, parsed.machineRef, effectiveProvider)
    }
    const worktreeId = await resolveLocalPlacement({
      directory: parsed.directory,
      machineRef: parsed.machineRef,
      kernelRef: parsed.kernelRef,
      label: "agent working directory",
    }, {
      baseDirectory: deps.currentWorktreeTarget(),
    })
    const sliceRef = await prepareSliceForSpawn(deps, parsed.sliceRef, parsed.sliceDisplayMode, worktreeId, remoteRef)
    const payload = await spawnAndLaunchAgent(deps, {
      provider,
      alias,
      model: model ?? null,
      effort,
      worktreeId,
      machineRef: sliceRef ? undefined : remoteRef,
      worktreePlacement,
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
  const ready = kernels.filter((kernel) => remoteKernelReadiness(kernel) === "ready")
  if (ready.length === 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "blocked")) {
    const kernelTargets = formatKernelTargets(kernels)
    throw new Error(`remote machine ${machineRef} has no kernel accepting remote agents; next: enable remote leases on ${kernelTargets} or choose another worker`)
  }
  if (ready.length === 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "needs-provider")) {
    const kernelTargets = formatKernelTargets(kernels)
    throw new Error(`remote machine ${machineRef} has no accepting kernel with provider CLIs; next: configure provider CLIs on ${kernelTargets} or choose another worker`)
  }
  if (ready.length === 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "needs-account")) {
    const kernelTargets = formatKernelTargets(kernels)
    throw new Error(`remote machine ${machineRef} has no ready worker kernel with authenticated provider accounts; next: configure/import or refresh provider accounts on ${kernelTargets} or choose another worker`)
  }
  if (ready.length === 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "unknown")) {
    throw new Error(`remote machine ${machineRef} has no kernel with known remote readiness; next: run /machine kernels ${machineRef}, refresh relay inventory, or choose another worker`)
  }
  if (ready.length === 0) {
    throw new Error(`remote machine ${machineRef} has no ready worker kernel; next: run /machine kernels ${machineRef}, fix the listed readiness issue, or choose another worker`)
  }
  if (!ready.some((kernel) => (kernel.available_providers ?? []).includes(provider))) {
    const providerCandidates = kernels.filter((kernel) => (
      kernel.accepting_remote_leases === true
      && (kernel.available_providers ?? []).includes(provider)
    ))
    if (
      providerCandidates.length > 0
      && providerCandidates.every((kernel) => remoteKernelReadiness(kernel) === "needs-account")
    ) {
      const kernelTargets = formatKernelTargets(providerCandidates)
      throw new Error(`remote machine ${machineRef} has no ready worker kernel with an authenticated ${provider} account; next: configure/import or refresh the ${provider} account on ${kernelTargets} or choose another worker`)
    }
    throw new Error(`remote machine ${machineRef} has no accepting kernel with provider ${provider}; next: choose a worker with ${provider} or change the agent provider`)
  }
}

function formatKernelTargets(kernels: readonly { kernel_id?: string | null; kernel_alias?: string | null; relay_alias?: string | null }[]): string {
  const labels = kernels
    .map((kernel) => kernel.relay_alias || kernel.kernel_alias || kernel.kernel_id)
    .filter((label): label is string => Boolean(label))
  if (labels.length === 0) {
    return "the listed worker kernel"
  }
  if (labels.length === 1) {
    return `kernel ${labels[0]}`
  }
  return `kernels ${labels.join(",")}`
}

async function prepareSliceForSpawn(
  deps: AgentSpawnCommandHandlerDeps,
  sliceSelection: string | undefined,
  sliceDisplayMode: "headless" | "headed" | undefined,
  worktreeId: string | undefined,
  workerKernelRef: string | undefined,
): Promise<string | undefined> {
  if (!sliceSelection || sliceSelection === "off") {
    return undefined
  }
  const createMode = sliceCreateMode(sliceSelection, sliceDisplayMode)
  if (!createMode) {
    if (!deps.startSlice) {
      throw new Error("slice reuse is unavailable in this build")
    }
    const slice = await deps.startSlice(sliceSelection)
    validateReusableSliceForSpawn(slice, {
      requestedRef: sliceSelection,
      workspaceId: deps.currentWorkspaceTarget(),
      worktreeId: worktreeId || deps.currentWorktreeTarget(),
      workerKernelRef,
    })
    return slice.id
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
    ...(workerKernelRef ? { workerKernelRef } : {}),
  })
  const started = await deps.startSlice(slice.id)
  return started.id
}

function sliceCreateMode(value: string, displayMode: "headless" | "headed" | undefined): { displayMode: "headless" | "headed" } | null {
  if (value === "new" || value === "new:headless") {
    return { displayMode: displayMode ?? "headless" }
  }
  if (value === "new:headed") return { displayMode: "headed" }
  return null
}

function defaultSliceName(worktreePath: string): string {
  const leaf = worktreePath.split("/").filter(Boolean).pop() || "workspace"
  const suffix = Date.now().toString(36).slice(-5)
  return `${leaf}-slice-${suffix}`.replace(/[^a-zA-Z0-9_.-]/g, "-")
}

function validateReusableSliceForSpawn(
  slice: SliceRecord,
  expected: {
    requestedRef: string
    workspaceId: string
    worktreeId: string
    workerKernelRef?: string | undefined
  },
): void {
  const label = slice.name || slice.id || expected.requestedRef
  if (slice.status !== "running") {
    throw new Error(`slice ${label} is ${slice.status}; next: run /slice status ${expected.requestedRef}, /slice logs ${expected.requestedRef}, then retry after it is running`)
  }
  if (slice.workspace_id && expected.workspaceId && slice.workspace_id !== expected.workspaceId) {
    throw new Error(`slice ${label} is scoped to workspace ${slice.workspace_id}, not ${expected.workspaceId}; choose a slice for this workspace, use --slice new, or use --slice off`)
  }
  const sliceWorktree = slice.worktree_id || slice.workspace_mount || ""
  if (sliceWorktree && expected.worktreeId && sliceWorktree !== expected.worktreeId) {
    throw new Error(`slice ${label} is scoped to worktree ${sliceWorktree}, not ${expected.worktreeId}; choose a slice for this worktree, use --slice new, or use --slice off`)
  }
  if (expected.workerKernelRef && !sliceMatchesWorkerRef(slice, expected.workerKernelRef)) {
    throw new Error(`slice ${label} runs on ${formatSliceWorker(slice)}, not ${expected.workerKernelRef}; choose a slice on that worker, use --slice new, or use --slice off`)
  }
}

function sliceMatchesWorkerRef(slice: SliceRecord, workerRef: string): boolean {
  return [
    slice.worker_kernel_id,
    slice.worker_kernel_ref,
    slice.worker_machine_id,
  ].some((candidate) => candidate === workerRef)
}

function formatSliceWorker(slice: SliceRecord): string {
  return slice.worker_machine_id || slice.worker_kernel_id || slice.worker_kernel_ref || "unknown worker"
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
    `spawned ${agent.meta_mode ? "meta-mode agent" : "agent"} ${agent.agent_ref}${alias ? ` (${alias})` : ""}`,
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
