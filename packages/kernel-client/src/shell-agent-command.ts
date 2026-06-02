import type {
  AgentInstance,
  RelayKernelPresence,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./kernel-types.js"
import {
  aliasAgentRequest,
  cycleAgentFocusRequest,
  focusAgentRequest,
  getProviderRunRequest,
  getSessionStateRequest,
  launchProviderRunRequest,
  listRemoteMachineKernelsRequest,
  listSlicesRequest,
  spawnAgentRequest,
  updateAgentConfigRequest,
  updateAgentProfileRequest,
  updateAgentSubstitutesRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatAgentInspectSummary,
  formatAgentListSummary,
  formatAgentRef,
  formatAgentSubstituteSummary,
} from "./shell-agent-format.js"
import {
  parseExecutionMode,
  parsePermissionLevel,
} from "./shell-agent-policy.js"
import { resolveShellAgent } from "./shell-agent-resolver.js"
import {
  parsePlacementOptions,
  resolveShellPlacement,
  type ShellPlacementDeps,
} from "./shell-placement.js"
import {
  resolveShellSliceRef,
  shellSliceCreatesPlacement,
} from "./shell-slice-placement.js"
import { remoteKernelReadiness } from "./shell-remote-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellAgentCommandDeps = ShellPlacementDeps & {
  client: ShellKernelClient
}

export async function executeAgentCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellAgentCommandDeps,
): Promise<ShellCommandResult> {
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }

  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const session = await getShellSessionState(deps, sessionId)
      const agents = session.agents
      const { slices } = agents.some((agent) => agent.remote_execution)
        ? await listAgentInspectSlices(deps)
        : { slices: [] }
      const providerRunContext = await activeProviderRunContext(deps, session)
      return {
        ok: true,
        message: formatAgentListSummary(agents, slices, providerRunContext, {
          homeKernelId: session.host_daemon_id ?? null,
          homeMachineId: session.host_machine_id ?? null,
          ownerUserId: session.owner_user_id ?? null,
          workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
        }),
        data: { agents, slices, session, providerRunContext },
      }
    }
    case "inspect":
    case "info":
    case "show": {
      const resolved = await resolveShellAgent(context, deps, args[0] ?? context.agentId)
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent inspect [agent-ref]" }
      }
      const session = await getShellSessionState(deps, sessionId)
      const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
      const { slices, error } = resolved.agent.remote_execution
        ? await listAgentInspectSlices(deps)
        : { slices: [], error: null }
      const providerRunContext = await activeProviderRunContext(deps, session)
      return {
        ok: true,
        message: formatAgentInspectSummary(agent, slices, error, providerRunContext, {
          homeKernelId: session.host_daemon_id ?? null,
          homeMachineId: session.host_machine_id ?? null,
          ownerUserId: session.owner_user_id ?? null,
          workspaceLiveSyncMode: session.workspace_live_sync_mode ?? null,
        }),
        data: { agent, slices, session, providerRunContext },
      }
    }
    case "spawn": {
      const parsedSpawn = parsePlacementOptions(args, true)
      if (parsedSpawn.error) {
        return { ok: false, message: parsedSpawn.error }
      }
      const [alias, model] = parsedSpawn.options.positional
      if (parsedSpawn.options.positional.length > 2) {
        return { ok: false, message: "usage: agent spawn [alias] [model] [--dir <directory>] [--worktree <directory> --branch <branch>] [--machine <machine-ref>|--kernel <kernel-ref>] [--slice off|new:headless|new:headed|<slice-ref>]" }
      }
      const resolvedMachineKernel = await resolveMachineSpawnKernelRef(parsedSpawn.options.machineRef, context.provider, deps)
      if (!resolvedMachineKernel.ok) {
        return { ok: false, message: resolvedMachineKernel.message }
      }
      const remoteKernelRef = parsedSpawn.options.kernelRef ?? resolvedMachineKernel.kernelRef
      if (remoteKernelRef && (parsedSpawn.options.directory || parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)) {
        return { ok: false, message: "usage: agent spawn [alias] [model] --machine/--kernel <ref> uses the worker kernel default directory" }
      }
      if (
        parsedSpawn.options.sliceRef
        && !shellSliceCreatesPlacement(parsedSpawn.options.sliceRef)
        && parsedSpawn.options.sliceRef !== "off"
        && (parsedSpawn.options.directory || parsedSpawn.options.gitWorktree || parsedSpawn.options.branch || parsedSpawn.options.fromRef)
      ) {
        return { ok: false, message: "usage: agent spawn [alias] [model] --slice <slice-ref> does not accept --dir or --worktree" }
      }
      const worktree = await resolveShellPlacement(parsedSpawn.options, context.worktree, "agent working directory", deps)
      const effectiveWorktree = worktree ?? context.worktree
      const sliceRef = await resolveShellSliceRef(
        parsedSpawn.options.sliceRef,
        context,
        effectiveWorktree,
        deps,
        parsedSpawn.options.sliceDisplayMode,
        remoteKernelRef,
      )
      const response = await deps.client.send(spawnAgentRequest(
        sessionId,
        context.provider,
        alias,
        model ?? context.model,
        worktree,
        context.effort,
        undefined,
        undefined,
        sliceRef ? undefined : remoteKernelRef,
        undefined,
        sliceRef,
      ))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentSpawned").agent
      const placement = agent.remote_execution
        ? sliceRef
          ? ` in slice ${sliceRef}`
          : ` on ${remoteKernelRef ?? agent.remote_execution.worker_machine_id}`
        : agent.worktree_id ? ` in ${agent.worktree_id}` : ""
      return resourceResult(
        `spawned agent ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}${placement}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "focus": {
      const agentRef = args[0]
      if (!agentRef) {
        return { ok: false, message: "usage: agent focus <agent-id>" }
      }
      const response = await deps.client.send(focusAgentRequest(sessionId, agentRef))
      const agent = expectVariant<{ agent: AgentInstance }>(response, "AgentFocused").agent
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "cycle": {
      const response = await deps.client.send(cycleAgentFocusRequest(sessionId))
      const agent = expectVariant<{ agent: AgentInstance | null }>(response, "AgentFocusCycled").agent
      if (!agent) {
        return { ok: true, message: "no agents to cycle" }
      }
      return resourceResult(
        `current agent = ${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""}`,
        parsed.assignment,
        agent.id,
        { agentId: agent.id },
        { agent },
      )
    }
    case "alias":
    case "name": {
      const reference = args.length > 1 ? args[0] : context.agentId
      const rawAlias = (args.length > 1 ? args.slice(1) : args).join(" ").trim()
      const resolved = await resolveShellAgent(context, deps, reference)
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent alias [agent-ref] <alias|clear>" }
      }
      if (!rawAlias) {
        return { ok: true, message: `${formatAgentRef(resolved.agent)} alias = ${resolved.agent.alias ?? "<none>"}`, data: { agent: resolved.agent } }
      }
      const shouldClearAgentAlias = rawAlias === "clear" || rawAlias === "none" || rawAlias === "-"
      const response = await deps.client.send(aliasAgentRequest(
        sessionId,
        resolved.agent.id,
        shouldClearAgentAlias ? "" : rawAlias,
      ))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentAliased")
      return { ok: true, message: `${formatAgentRef(payload.agent)} alias = ${payload.agent.alias ?? "<none>"}`, data: payload }
    }
    case "provider":
    case "model":
    case "variant": {
      const resolved = await resolveShellAgent(context, deps, args.length > 1 ? args[0] : context.agentId)
      const rawValue = args.length > 1 ? args.slice(1).join(" ").trim() : args.join(" ").trim()
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : `usage: agent ${action} [agent-ref] <value>` }
      }
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const value = action === "provider"
          ? agent.provider
          : action === "model"
            ? agent.model ?? "<none>"
            : agent.effort ?? "<none>"
        return { ok: true, message: `${formatAgentRef(agent)} ${action} = ${value}`, data: { session, agent } }
      }
      const shouldClearEffort = action === "variant" && ["clear", "none", "-", "default"].includes(rawValue)
      const response = await deps.client.send(updateAgentProfileRequest({
        sessionId,
        agentId: resolved.agent.id,
        ...(action === "provider" ? { provider: rawValue } : {}),
        ...(action === "model" ? { model: rawValue } : {}),
        ...(action === "variant" && !shouldClearEffort ? { effort: rawValue } : {}),
        ...(shouldClearEffort ? { clearEffort: true } : {}),
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentProfileUpdated")
      const value = action === "provider"
        ? payload.agent.provider
        : action === "model"
          ? payload.agent.model ?? "<none>"
          : payload.agent.effort ?? "<none>"
      return { ok: true, message: `${formatAgentRef(payload.agent)} ${action} = ${value}`, data: payload }
    }
    case "mode": {
      const firstArgIsMode = args[0] === "inherit" || parseExecutionMode(args[0]) != null
      const resolved = await resolveShellAgent(context, deps, firstArgIsMode ? context.agentId : args[0])
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent mode [agent-ref] <build|plan|inherit>" }
      }
      const rawValue = args.length > 1 ? args[1] : firstArgIsMode ? args[0] : undefined
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const sessionMode = parseExecutionMode(session.config_state?.values?.["agents.mode"]) ?? "build"
        const effectiveMode = agent.execution_mode_override ?? sessionMode
        const source = agent.execution_mode_override ? "agent" : "session"
        return { ok: true, message: `${formatAgentRef(agent)} mode = ${effectiveMode} (${source})`, data: { session, agent } }
      }
      if (rawValue !== "inherit" && !parseExecutionMode(rawValue)) {
        return { ok: false, message: "usage: agent mode [agent-ref] <build|plan|inherit>" }
      }
      const response = await deps.client.send(updateAgentConfigRequest({
        sessionId,
        agentId: resolved.agent.id,
        executionMode: rawValue === "inherit" ? null : parseExecutionMode(rawValue),
        clearExecutionMode: rawValue === "inherit",
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
      const sessionMode = parseExecutionMode(payload.session.config_state?.values?.["agents.mode"]) ?? "build"
      const effectiveMode = payload.agent.execution_mode_override ?? sessionMode
      return { ok: true, message: `${formatAgentRef(payload.agent)} mode = ${effectiveMode}${rawValue === "inherit" ? " (session)" : " (agent)"}`, data: payload }
    }
    case "permissions": {
      const firstArgIsPermission = args[0] === "inherit" || parsePermissionLevel(args[0]) != null
      const resolved = await resolveShellAgent(context, deps, firstArgIsPermission ? context.agentId : args[0])
      if (!resolved.ok) {
        return { ok: false, message: args[0] ? resolved.message : "usage: agent permissions [agent-ref] <required|yolo|inherit>" }
      }
      const rawValue = args.length > 1 ? args[1] : firstArgIsPermission ? args[0] : undefined
      if (!rawValue) {
        const response = await deps.client.send(getSessionStateRequest(sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        const agent = session.agents.find((entry) => entry.id === resolved.agent.id) ?? resolved.agent
        const sessionLevel = parsePermissionLevel(session.config_state?.values?.["agents.permissions"]) ?? "yolo"
        const effectiveLevel = agent.permission_level_override ?? sessionLevel
        const source = agent.permission_level_override ? "agent" : "session"
        return { ok: true, message: `${formatAgentRef(agent)} permissions = ${effectiveLevel} (${source})`, data: { session, agent } }
      }
      if (rawValue !== "inherit" && !parsePermissionLevel(rawValue)) {
        return { ok: false, message: "usage: agent permissions [agent-ref] <required|yolo|inherit>" }
      }
      const response = await deps.client.send(updateAgentConfigRequest({
        sessionId,
        agentId: resolved.agent.id,
        permissionLevel: rawValue === "inherit" ? null : parsePermissionLevel(rawValue),
        clearPermissionLevel: rawValue === "inherit",
      }))
      const payload = expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
      const sessionLevel = parsePermissionLevel(payload.session.config_state?.values?.["agents.permissions"]) ?? "yolo"
      const effectiveLevel = payload.agent.permission_level_override ?? sessionLevel
      return { ok: true, message: `${formatAgentRef(payload.agent)} permissions = ${effectiveLevel}${rawValue === "inherit" ? " (session)" : " (agent)"}`, data: payload }
    }
    case "substitute":
    case "subs":
      return executeAgentSubstituteCommand(args, context, deps, sessionId)
    default:
      return { ok: false, message: "usage: agent list|inspect|spawn|focus|cycle|alias|provider|model|variant|mode|permissions|substitute" }
  }
}

async function resolveMachineSpawnKernelRef(
  machineRef: string | undefined,
  provider: string,
  deps: ShellAgentCommandDeps,
): Promise<{ readonly ok: true; readonly kernelRef?: string } | { readonly ok: false; readonly message: string }> {
  if (!machineRef) {
    return { ok: true }
  }
  const response = await deps.client.send(listRemoteMachineKernelsRequest(machineRef))
  const kernels = expectVariant<{ kernels: RelayKernelPresence[] }>(response, "RemoteMachineKernelsListed").kernels
  if (kernels.length === 0) {
    return { ok: false, message: `remote machine ${machineRef} has no live worker kernels; next: run machine kernels ${machineRef} or choose another worker` }
  }
  const ready = kernels.filter((kernel) => remoteKernelReadiness(kernel) === "ready")
  if (ready.length === 0) {
    return { ok: false, message: `remote machine ${machineRef} has no ready worker kernel; next: run machine kernels ${machineRef}, fix the listed readiness issue, or choose another worker` }
  }
  const providerReady = ready.find((kernel) => (kernel.available_providers ?? []).includes(provider))
  if (!providerReady) {
    return { ok: false, message: `remote machine ${machineRef} has no accepting kernel with provider ${provider}; next: choose a worker with ${provider} or change the agent provider` }
  }
  return { ok: true, kernelRef: providerReady.kernel_id }
}

async function listAgentInspectSlices(deps: ShellAgentCommandDeps): Promise<{
  slices: SliceRecord[]
  error: string | null
}> {
  try {
    const response = await deps.client.send(listSlicesRequest())
    return {
      slices: expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices,
      error: null,
    }
  } catch (error) {
    return {
      slices: [],
      error: error instanceof Error ? error.message : "slice inventory unavailable",
    }
  }
}

async function getShellSessionState(
  deps: ShellAgentCommandDeps,
  sessionId: string,
): Promise<RuntimeSession> {
  const response = await deps.client.send(getSessionStateRequest(sessionId))
  return expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
}

async function activeProviderRunContext(
  deps: ShellAgentCommandDeps,
  session: RuntimeSession,
): Promise<{
  activeProviderRunId?: string | null
  activeProviderRunAgentId?: string | null
  activeProviderRunLookupError?: string | null
}> {
  if (!session.active_provider_run_id) {
    return {}
  }
  try {
    const response = await deps.client.send(getProviderRunRequest(session.active_provider_run_id))
    const providerRun = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun").provider_run
    return {
      activeProviderRunId: providerRun.id,
      activeProviderRunAgentId: providerRun.agent_instance_id ?? null,
    }
  } catch (error) {
    return {
      activeProviderRunId: session.active_provider_run_id,
      activeProviderRunAgentId: null,
      activeProviderRunLookupError: error instanceof Error ? error.message : "provider run lookup failed",
    }
  }
}

async function executeAgentSubstituteCommand(
  args: string[],
  context: ShellContext,
  deps: ShellAgentCommandDeps,
  sessionId: string,
): Promise<ShellCommandResult> {
  const [subcommand = "list", ...rawArgs] = args
  const agentFlagIndex = rawArgs.indexOf("--agent")
  const agentRefFromFlag = agentFlagIndex >= 0 ? rawArgs[agentFlagIndex + 1] : undefined
  const filteredArgs = agentFlagIndex >= 0
    ? rawArgs.filter((_, index) => index !== agentFlagIndex && index !== agentFlagIndex + 1)
    : rawArgs
  const resolved = await resolveShellAgent(context, deps, agentRefFromFlag)
  if (!resolved.ok) {
    return { ok: false, message: resolved.message }
  }
  const agent = resolved.agent
  if (subcommand === "list" || subcommand === "ls") {
    return { ok: true, message: formatAgentSubstituteSummary(agent), data: { agent } }
  }
  const update = async (action: Record<string, unknown>) => {
    const response = await deps.client.send(updateAgentSubstitutesRequest({
      sessionId,
      agentId: agent.id,
      action: action as never,
    }))
    return expectVariant<{ agent: AgentInstance; session: RuntimeSession }>(response, "AgentConfigUpdated")
  }
  if (subcommand === "add") {
    const provider = filteredArgs[0]
    const model = filteredArgs[1]
    const variantIndex = filteredArgs.indexOf("--variant")
    const variant = variantIndex >= 0 ? filteredArgs[variantIndex + 1] : undefined
    const kernelIndex = filteredArgs.indexOf("--kernel")
    const kernelId = kernelIndex >= 0 ? filteredArgs[kernelIndex + 1] : undefined
    const worktreeIndex = filteredArgs.indexOf("--worktree")
    const worktreeId = worktreeIndex >= 0 ? filteredArgs[worktreeIndex + 1] : undefined
    if (!provider || !model) {
      return { ok: false, message: "usage: agent substitute add <provider> <model> [--variant v] [--kernel k] [--worktree dir] [--agent a]" }
    }
    const payload = await update({
      Add: {
        provider,
        model,
        variant: variant ?? null,
        kernel_id: kernelId ?? null,
        worktree_id: worktreeId ?? null,
      },
    })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute added: ${provider}/${model}${variant ? `/${variant}` : ""}`, data: payload }
  }
  if (subcommand === "remove" || subcommand === "rm") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      return { ok: false, message: "usage: agent substitute remove <index> [--agent a]" }
    }
    const payload = await update({ Remove: { index } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute ${index} removed`, data: payload }
  }
  if (subcommand === "clear") {
    const payload = await update({ Clear: {} })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitutes cleared`, data: payload }
  }
  if (subcommand === "timeout") {
    const timeoutMs = parseSubstitutionTimeoutMs(filteredArgs[0])
    if (timeoutMs === undefined && filteredArgs[0] !== "inherit" && filteredArgs[0] !== "default") {
      return { ok: false, message: "usage: agent substitute timeout <ms|Ns|inherit> [--agent a]" }
    }
    const payload = await update({ SetTimeout: { timeout_ms: timeoutMs ?? null } })
    return { ok: true, message: `${formatAgentRef(payload.agent)} substitute timeout: ${timeoutMs == null ? "default" : `${timeoutMs}ms`}`, data: payload }
  }
  if (subcommand === "activate") {
    const index = Number.parseInt(filteredArgs[0] ?? "", 10)
    if (!Number.isFinite(index)) {
      return { ok: false, message: "usage: agent substitute activate <index> [--agent a]" }
    }
    const payload = await update({ Activate: { index, reason: "manual" } })
    const profile = payload.agent.substitutes?.[index]
    if (!profile) {
      return { ok: false, message: `${formatAgentRef(payload.agent)} substitute ${index} is not available`, data: payload }
    }
    const response = await deps.client.send(launchProviderRunRequest(
      sessionId,
      profile.provider,
      "default",
      profile.model,
      profile.variant ?? "",
      payload.agent.id,
    ))
    return { ok: true, message: `${formatAgentRef(payload.agent)} activated substitute ${index}: ${profile.provider}/${profile.model}`, data: { ...payload, launch: response }, contextUpdates: { agentId: payload.agent.id } }
  }
  if (subcommand === "primary") {
    const payload = await update({ Primary: {} })
    const response = await deps.client.send(launchProviderRunRequest(
      sessionId,
      payload.agent.provider,
      "default",
      payload.agent.model ?? context.model,
      payload.agent.effort ?? context.effort,
      payload.agent.id,
    ))
    return { ok: true, message: `${formatAgentRef(payload.agent)} returned to primary profile`, data: { ...payload, launch: response }, contextUpdates: { agentId: payload.agent.id } }
  }
  return { ok: false, message: "usage: agent substitute list|add|remove|clear|timeout|activate|primary" }
}

function resourceResult(
  message: string,
  assignment: string | undefined,
  value: string,
  contextUpdates: ShellCommandResult["contextUpdates"],
  data: unknown,
): ShellCommandResult {
  return {
    ok: true,
    message,
    data,
    bindings: assignment ? { [assignment]: value } : undefined,
    contextUpdates,
  }
}

function parseSubstitutionTimeoutMs(value: string | null | undefined): number | undefined {
  if (!value || value === "inherit" || value === "default") return undefined
  const normalized = value.trim().toLowerCase()
  const match = normalized.match(/^(\d+)(ms|s|m)?$/)
  if (!match) return undefined
  const amount = Number.parseInt(match[1] ?? "", 10)
  const unit = match[2] ?? "ms"
  if (!Number.isFinite(amount)) return undefined
  if (unit === "m") return amount * 60_000
  if (unit === "s") return amount * 1_000
  return amount
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
