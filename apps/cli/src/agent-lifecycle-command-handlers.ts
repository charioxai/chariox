import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "./cli-types.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import { responsePaneBindingsMatch, selectResponsePaneAgents } from "./response-panes.js"
import type { ResolvedAgentReference } from "./session-agent-resolver.js"
import { formatSliceAuthIdentity, formatSliceScope } from "./slice-format.js"

type FooterTone = "info" | "error"

type AgentCyclePayload = {
  agent: AgentInstance | null
  session: RuntimeSession
}

type AgentFocusPayload = {
  agent: AgentInstance
  session: RuntimeSession
}

type AgentProviderRunContext = {
  activeProviderRunId?: string | null
  activeProviderRunAgentId?: string | null
}

type AgentSessionContext = {
  homeKernelId?: string | null
  homeMachineId?: string | null
  ownerUserId?: string | null
  workspaceLiveSyncMode?: RuntimeSession["workspace_live_sync_mode"]
}

export type AgentLifecycleCommandHandlerDeps = {
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  currentModelId: () => string
  currentVariantId: () => string
  providerRunState: () => RuntimeProviderRun | null
  multiAgentResponseLayout: () => MultiAgentResponseLayout
  maxAgentsPerScreen: () => number
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  listSlices?: () => Promise<SliceRecord[]>
  formatError: (error: unknown) => string
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  rebuildTranscript: () => void
  cycleAgentFocus: () => Promise<AgentCyclePayload>
  launchAgentProviderRun: (
    provider: string,
    model: string,
    variant: string,
    agentId: string,
  ) => Promise<RuntimeProviderRun>
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  refreshSessionState: (sessionId: string) => Promise<RuntimeSession>
  destroyAgent: (agentId: string) => Promise<RuntimeSession>
  focusAgent: (agentId: string) => Promise<AgentFocusPayload>
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  formatAgentLabel: (agent: AgentInstance | null | undefined) => string
  refreshSplitPaneFocusRepaint: () => void
}

export async function handleCycleAgentFocus(
  deps: AgentLifecycleCommandHandlerDeps,
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("must be attached to a session to cycle agents", "error")
    return
  }
  try {
    const previousSession = deps.sessionState()
    const payload = await deps.cycleAgentFocus()
    await applyFocusedAgentSession(deps, previousSession, payload.session, payload.agent)
    if (payload.agent) {
      deps.flashFooter(
        `cycled to agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
        "info",
      )
    } else {
      deps.flashFooter("no agents to cycle", "info")
    }
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

export async function handleAgentDeleteCommand(
  deps: AgentLifecycleCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const reference = args[1]
  const resolved = deps.resolveSessionAgent(reference)
  if (resolved.error || !resolved.agent) {
    deps.flashFooter(resolved.error ?? "usage: /agent delete <agent-name|agent-alias>", "error")
    return
  }
  try {
    const nextSession = await deps.destroyAgent(resolved.agent.id)
    deps.applySessionState(nextSession)
    await deps.refreshAgentPanes(nextSession)
    deps.rebuildTranscript()
    deps.refreshSplitPaneFocusRepaint()
    deps.flashFooter(`deleted agent ${deps.formatAgentLabel(resolved.agent)}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

export async function handleAgentFocusCommand(
  deps: AgentLifecycleCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const agentId = args[1]
  if (!agentId) {
    deps.flashFooter("usage: /agent focus <agent-id>", "error")
    return
  }
  try {
    const previousSession = deps.sessionState()
    const payload = await deps.focusAgent(agentId)
    await applyFocusedAgentSession(deps, previousSession, payload.session, payload.agent)
    deps.flashFooter(
      `focused on agent ${payload.agent.agent_ref}${payload.agent.alias ? ` (${payload.agent.alias})` : ""}`,
      "info",
    )
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
}

export function formatAgentListSummary(
  agents: AgentInstance[],
  slices: readonly SliceRecord[] = [],
  providerRunContext: AgentProviderRunContext = {},
): string {
  if (agents.length === 0) {
    return "no agents in session"
  }
  const agentList = agents
    .map((agent) => formatAgentListEntry(agent, slices, providerRunContext))
    .join(", ")
  return `${agents.length} agent${agents.length === 1 ? "" : "s"}: ${agentList}`
}

export function formatAgentInspectSummary(
  agent: AgentInstance,
  slices: readonly SliceRecord[] = [],
  providerRunContext: AgentProviderRunContext = {},
  sessionContext: AgentSessionContext = {},
): string {
  const slice = sliceForRemoteAgent(agent, slices)
  const lines = [
    `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${agent.state}]`,
    `id: ${agent.id}`,
    `session: ${agent.session_id}`,
    `home kernel: ${formatHomeKernel(sessionContext)}`,
    `session owner: ${sessionContext.ownerUserId || "<unknown>"}`,
    `live sync: ${formatSessionWorkspaceLiveSyncMode(sessionContext.workspaceLiveSyncMode)}`,
    `provider: ${agent.provider}`,
    `model: ${agent.model ?? "<none>"}`,
    `variant: ${agent.effort ?? "<none>"}`,
    `mode: ${agent.execution_mode_override ?? "session"}`,
    `permissions: ${agent.permission_level_override ?? "session"}`,
    `worktree: ${agent.worktree_id ?? "<none>"}`,
    `placement: ${formatAgentInspectPlacement(agent, slice)}`,
    `provider run: ${formatAgentProviderRunSummary(agent, providerRunContext)}`,
    ...(slice ? [
      `slice: ${formatSliceInspectSummary(slice)}`,
      `slice provider accounts: ${formatSliceProviderAccounts(slice)}`,
    ] : []),
    `extensions: ${formatAgentInspectExtensionSummary(agent)}`,
    `remote extension sync: ${formatAgentInspectRemoteExtensionSync(agent)}`,
    `substitutes: ${formatAgentInspectSubstitutes(agent)}`,
  ]
  if (agent.active_substitute_index != null) {
    lines.push(`active substitute: ${agent.active_substitute_index}`)
  }
  if (agent.last_substitution) {
    lines.push(`last substitution: ${agent.last_substitution.reason}`)
  }
  lines.push(`created: ${formatTimestamp(agent.created_at_ms)}`)
  lines.push(`last activity: ${formatTimestamp(agent.last_activity_at_ms)}`)
  return lines.join("\n")
}

function formatHomeKernel(context: AgentSessionContext): string {
  const homeKernel = context.homeKernelId || "<unknown>"
  return context.homeMachineId ? `${homeKernel}@${context.homeMachineId}` : homeKernel
}

function formatSessionWorkspaceLiveSyncMode(
  mode: AgentSessionContext["workspaceLiveSyncMode"],
): string {
  if (mode === "managed" || mode === "tracked") {
    return `${mode} (selected workspace/worktree only; other repositories unrestricted)`
  }
  if (mode === "unrestricted") {
    return "off"
  }
  return "config default"
}

function formatAgentListEntry(
  agent: AgentInstance,
  slices: readonly SliceRecord[],
  providerRunContext: AgentProviderRunContext,
): string {
  return `${agent.agent_ref}${agent.alias ? ` (${agent.alias})` : ""} [${[
    agent.state,
    formatAgentProvider(agent),
    `worktree ${agent.worktree_id ?? "-"}`,
    formatAgentPlacement(agent, sliceForRemoteAgent(agent, slices)),
    formatAgentListProviderRun(agent, providerRunContext),
    formatAgentGrantCount(agent),
    formatAgentRemoteExtensionSync(agent),
  ].filter(Boolean).join("; ")}]`
}

function formatAgentListProviderRun(
  agent: AgentInstance,
  context: AgentProviderRunContext,
): string | null {
  const runId = agentProviderRunId(agent, context)
  return runId ? `session run ${runId}` : null
}

function formatAgentProvider(agent: AgentInstance): string {
  const model = agent.primary_model ?? agent.model
  const provider = agent.primary_provider ?? agent.provider
  if (!model) {
    return provider
  }
  return model.startsWith(`${provider}/`) ? model : `${provider} ${model}`
}

function formatAgentPlacement(agent: AgentInstance, slice: SliceRecord | null): string {
  const remote = agent.remote_execution
  if (!remote) {
    return "local"
  }
  if (slice) {
    const run = remote.active_worker_provider_run_id ? ` run ${remote.active_worker_provider_run_id}` : ""
    return `slice ${slice.name || slice.id}${run}`
  }
  const machine = remote.worker_machine_id ? `@${remote.worker_machine_id}` : ""
  const run = remote.active_worker_provider_run_id ? ` run ${remote.active_worker_provider_run_id}` : ""
  return `remote ${remote.worker_kernel_id}${machine}${run}`
}

function formatAgentRemoteExtensionSync(agent: AgentInstance): string {
  const sync = agent.remote_extension_manifest_sync
  if (!sync) {
    return agent.remote_execution ? `manifest pending next ${formatAgentListRemoteExtensionSyncAction(agent)}` : ""
  }
  const hash = sync.manifest_hash ? ` ${sync.manifest_hash.slice(0, 8)}` : ""
  const revoke = sync.pending_revoke ? " pending revoke" : ""
  const error = sync.last_error ? ` error ${sync.last_error}` : ""
  const action = sync.state === "failed" || sync.state === "stale" || sync.pending_revoke || sync.last_error
    ? ` next ${formatAgentListRemoteExtensionSyncAction(agent)}`
    : ""
  return `manifest ${sync.state}${hash}${revoke}${error}${action}`
}

function formatAgentListRemoteExtensionSyncAction(agent: AgentInstance): string {
  const worker = agent.remote_execution?.worker_machine_id
    ? `; /machine kernels ${agent.remote_execution.worker_machine_id}`
    : ""
  return `/extension sync-status ${agent.agent_ref}${worker}; /extension sync-retry ${agent.agent_ref}`
}

function formatAgentGrantCount(agent: AgentInstance): string {
  const grants = agent.extension_grants?.length ?? 0
  const count = `${grants} grant${grants === 1 ? "" : "s"}`
  return grants > 0 ? `${count} (${formatAgentExtensionPlacementSummary(agent)})` : count
}

function formatAgentInspectPlacement(agent: AgentInstance, slice: SliceRecord | null): string {
  const remote = agent.remote_execution
  if (!remote) {
    return "worker-local"
  }
  const worker = remote.worker_machine_id || remote.worker_kernel_id
  const placement = slice ? `slice ${slice.name || slice.id}` : "remote"
  const parts = [
    worker ? `worker=${worker}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `active_run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return `${placement}${parts.length > 0 ? ` (${parts.join(", ")})` : ""}`
}

function formatAgentProviderRunSummary(
  agent: AgentInstance,
  context: AgentProviderRunContext,
): string {
  const sessionRunId = agentProviderRunId(agent, context)
  const workerRunId = agent.remote_execution?.active_worker_provider_run_id ?? null
  if (!sessionRunId && context.activeProviderRunId && !context.activeProviderRunAgentId) {
    return [
      `session=${context.activeProviderRunId} owner unknown`,
      workerRunId ? `worker=${workerRunId}` : null,
    ].filter(Boolean).join(", ")
  }
  if (!sessionRunId && !workerRunId) {
    return "none"
  }
  return [
    sessionRunId ? `session=${sessionRunId}` : null,
    workerRunId ? `worker=${workerRunId}` : null,
  ].filter(Boolean).join(", ")
}

function agentProviderRunId(
  agent: AgentInstance,
  context: AgentProviderRunContext,
): string | null {
  if (!context.activeProviderRunId) {
    return null
  }
  if (context.activeProviderRunAgentId) {
    return context.activeProviderRunAgentId === agent.id ? context.activeProviderRunId : null
  }
  return null
}

function formatAgentInspectExtensionSummary(agent: AgentInstance): string {
  const grants = agent.extension_grants ?? []
  if (grants.length === 0) {
    return "none"
  }
  const counts = grants.reduce<Record<string, number>>((acc, grant) => {
    acc[grant.kind] = (acc[grant.kind] ?? 0) + 1
    return acc
  }, {})
  const byKind = ["mcp", "skill", "script", "connector"]
    .map((kind) => counts[kind] ? `${kind}=${counts[kind]}` : null)
    .filter(Boolean)
    .join(", ")
  const placement = formatAgentExtensionPlacementSummary(agent)
  return `${grants.length} grant${grants.length === 1 ? "" : "s"} (${placement}${byKind ? `; ${byKind}` : ""})`
}

function formatAgentExtensionPlacementSummary(agent: AgentInstance): string {
  if (!agent.remote_execution) {
    return "worker-local"
  }
  const grants = agent.extension_grants ?? []
  const activeHomeProxy = grants.some((grant) => grant.kind !== "skill")
  const passiveSkillSnapshot = grants.some((grant) => grant.kind === "skill")
  return [
    activeHomeProxy ? "active tools home-proxy" : null,
    passiveSkillSnapshot ? "skills snapshot" : null,
  ].filter(Boolean).join("; ") || "home-proxy"
}

function formatAgentInspectRemoteExtensionSync(agent: AgentInstance): string {
  if (!agent.remote_execution) {
    return "not applicable"
  }
  const sync = agent.remote_extension_manifest_sync
  if (!sync) {
    return `pending, next=${formatAgentRemoteExtensionSyncNextAction(agent)}`
  }
  const details = [
    sync.state,
    sync.pending_revoke ? "pending revoke" : null,
    sync.manifest_hash ? `hash=${sync.manifest_hash.slice(0, 12)}` : null,
    sync.last_error ? `error=${sync.last_error}` : null,
    sync.last_synced_at_ms ? `synced=${formatTimestamp(sync.last_synced_at_ms)}` : null,
    sync.last_attempted_at_ms ? `attempted=${formatTimestamp(sync.last_attempted_at_ms)}` : null,
  ].filter(Boolean)
  if (sync.state === "failed" || sync.state === "stale" || sync.pending_revoke || sync.last_error) {
    details.push(`next=${formatAgentRemoteExtensionSyncNextAction(agent)}`)
  }
  return details.join(", ")
}

function formatAgentRemoteExtensionSyncNextAction(agent: AgentInstance): string {
  const worker = agent.remote_execution?.worker_machine_id
    ? `; run /machine kernels ${agent.remote_execution.worker_machine_id}`
    : ""
  return `run /extension sync-status ${agent.agent_ref}${worker}; use /extension sync-retry ${agent.agent_ref} after worker connectivity is healthy`
}

function formatAgentInspectSubstitutes(agent: AgentInstance): string {
  const substitutes = agent.substitutes ?? []
  if (substitutes.length === 0) {
    return "none"
  }
  return substitutes.map((substitute, index) => {
    const marker = agent.active_substitute_index === index ? "*" : ""
    const variant = substitute.variant ? `/${substitute.variant}` : ""
    return `${marker}${index}:${substitute.provider}/${substitute.model}${variant}`
  }).join(", ")
}

function formatTimestamp(timestampMs: number | null | undefined): string {
  if (!timestampMs) {
    return "<none>"
  }
  return new Date(timestampMs).toISOString()
}

function sliceForRemoteAgent(
  agent: AgentInstance,
  slices: readonly SliceRecord[],
): SliceRecord | null {
  const remote = agent.remote_execution
  if (!remote) {
    return null
  }
  return slices.find((slice) => slice.agent_ids?.includes(agent.id))
    ?? slices.find((slice) =>
      slice.worker_kernel_id === remote.worker_kernel_id
      || slice.worker_kernel_ref === remote.worker_kernel_id
      || slice.worker_machine_id === remote.worker_machine_id,
    ) ?? null
}

function formatSliceInspectSummary(slice: SliceRecord): string {
  const worktree = formatSliceScope(slice)
  return `${slice.name || slice.id} (${[
    `id=${slice.id}`,
    `status=${slice.status}`,
    `display=${slice.display_mode ?? "headless"}`,
    `worktree=${worktree}`,
    `agents=${slice.agent_ids?.length ?? 0}`,
  ].join(", ")})`
}

function formatSliceProviderAccounts(slice: SliceRecord): string {
  const accounts = slice.provider_auth ?? []
  if (accounts.length === 0) {
    return "none"
  }
  return accounts.map((auth) => {
    return `${auth.provider}=${formatSliceAuthIdentity(auth)}`
  }).join(", ")
}

async function applyFocusedAgentSession(
  deps: AgentLifecycleCommandHandlerDeps,
  previousSession: RuntimeSession,
  nextSession: RuntimeSession,
  agent: AgentInstance | null,
) {
  const previousSelection = selectResponsePaneAgents(
    previousSession.agents,
    previousSession.focused_agent_id,
    deps.multiAgentResponseLayout() === "split",
    deps.maxAgentsPerScreen(),
  )
  const nextSelection = selectResponsePaneAgents(
    nextSession.agents,
    nextSession.focused_agent_id,
    deps.multiAgentResponseLayout() === "split",
    deps.maxAgentsPerScreen(),
  )
  const shouldRefreshPaneContents = deps.multiAgentResponseLayout() !== "split"
    || !responsePaneBindingsMatch(previousSelection, nextSelection)
  deps.applySessionState(nextSession)
  if (shouldRefreshPaneContents) {
    await deps.refreshAgentPanes(nextSession)
  }
  if (!nextSession.active_provider_run_id && agent) {
    const run = await deps.launchAgentProviderRun(
      agent.provider,
      agent.model ?? deps.currentModelId(),
      deps.currentVariantId(),
      agent.id,
    )
    deps.setProviderRunState(run)
    deps.applySessionState(await deps.refreshSessionState(nextSession.id))
  }
}
