import type { AgentInstance, RuntimeProviderRun, RuntimeSession, SliceRecord } from "./kernel-types.js"
import { getProviderRunRequest, getSessionStateRequest, listSlicesRequest } from "./ipc-requests.js"
import { formatRemoteExtensionSyncStatusLine, remoteExtensionSyncNextAction } from "./shell-capability-format.js"
import {
  formatExtensionAuthorityBoundaryDetail,
  formatExtensionGrantRuntimeDetail,
  formatExtensionGrantPlacementSummary,
  hasActiveHomeProxyExtensionGrants,
} from "./extension-grant-placement.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  parseExecutionMode,
  parsePermissionLevel,
} from "./shell-agent-policy.js"
import { sessionAgentIsBusy } from "./shell-agent-activity.js"
import { expectSessionState } from "./shell-session-attachment.js"
import { formatWorkspaceLiveSyncModeLabel } from "./workspace-live-sync-mode.js"
import { providerRunRecoveryActions } from "./provider-run-recovery.js"

type ShellContextCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeContextCommand(
  context: ShellContext,
  deps: ShellContextCommandDeps,
): Promise<ShellCommandResult> {
  let session: RuntimeSession | null = null
  let slices: SliceRecord[] = []
  let sliceLookupError: string | null = null
  let activeProviderRun: RuntimeProviderRun | null = null
  let providerRunLookupError: string | null = null
  if (context.sessionId) {
    try {
      const response = await deps.client.send(getSessionStateRequest(context.sessionId))
      session = expectSessionState(response)
    } catch {
      session = null
    }
  }
  if (session?.active_provider_run_id) {
    try {
      const response = await deps.client.send(getProviderRunRequest(session.active_provider_run_id))
      activeProviderRun = expectVariant<{ provider_run: RuntimeProviderRun }>(response, "ProviderRun").provider_run
    } catch (error) {
      providerRunLookupError = error instanceof Error ? error.message : "provider run lookup failed"
    }
  }
  const currentAgent = findCurrentAgent(context, session)
  if (currentAgent?.remote_execution) {
    try {
      const response = await deps.client.send(listSlicesRequest())
      slices = expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
    } catch (error) {
      sliceLookupError = error instanceof Error ? error.message : "slice lookup failed"
    }
  }
  return {
    ok: true,
    message: formatShellContext(context, session, slices, sliceLookupError, activeProviderRun, providerRunLookupError),
    data: { context, session, slices, activeProviderRun },
  }
}

function formatShellContext(
  context: ShellContext,
  session: RuntimeSession | null = null,
  slices: readonly SliceRecord[] = [],
  sliceLookupError: string | null = null,
  activeProviderRun: RuntimeProviderRun | null = null,
  providerRunLookupError: string | null = null,
): string {
  const currentAgent = findCurrentAgent(context, session)
  const currentAgentId = currentAgent?.id ?? context.agentId ?? null
  const currentAgentBusy = sessionAgentIsBusy(session, currentAgentId)
  const agentLabel = currentAgent
    ? `${currentAgent.agent_ref}${currentAgent.alias ? ` (${currentAgent.alias})` : ""}${currentAgentBusy ? " (busy)" : ""}`
    : `${context.agentId ?? "-"}${currentAgentBusy ? " (busy)" : ""}`
  const sessionMode = parseExecutionMode(session?.config_state?.values?.["agents.mode"]) ?? "build"
  const sessionPermissions = parsePermissionLevel(session?.config_state?.values?.["agents.permissions"]) ?? "yolo"
  const effectiveAgentMode = currentAgent?.execution_mode_override ?? sessionMode
  const effectiveAgentPermissions = currentAgent?.permission_level_override ?? sessionPermissions
  const lines = [
    `workspace: ${context.workspace}`,
    `worktree: ${context.worktree}`,
    `session: ${context.sessionId ?? "-"}`,
    `home kernel: ${formatContextHomeKernel(session)}`,
    `session owner: ${session?.owner_user_id?.trim() || "-"}`,
    `runtime authority: ${formatContextRuntimeAuthority(session)}`,
    `workspace live sync: ${formatWorkspaceLiveSyncModeLabel(session?.workspace_live_sync_mode)}`,
    `attachment: ${context.attachmentId ?? "-"}`,
    `agent: ${agentLabel}`,
    ...(currentAgent ? [
      `agent placement: ${formatContextAgentPlacement(currentAgent, slices, sliceLookupError)}`,
      `provider run: ${formatContextProviderRun(currentAgent, session, activeProviderRun, providerRunLookupError)}`,
      ...formatContextProviderRunNextAction(currentAgent, session, activeProviderRun),
      `extensions: ${formatContextExtensionSummary(currentAgent)}`,
      `extension runtime: ${formatExtensionGrantRuntimeDetail(currentAgent.extension_grants, Boolean(currentAgent.remote_execution))}`,
      `extension boundary: ${formatExtensionAuthorityBoundaryDetail(currentAgent.extension_grants, Boolean(currentAgent.remote_execution))}`,
      ...formatContextRemoteExtensionSyncLines(currentAgent),
    ] : []),
    `mode: ${currentAgent ? `${effectiveAgentMode} (agent${currentAgent.execution_mode_override ? "-override" : "-session"})` : sessionMode}`,
    `permissions: ${currentAgent ? `${effectiveAgentPermissions} (agent${currentAgent.permission_level_override ? "-override" : "-session"})` : sessionPermissions}`,
    `workflow: ${context.workflowId ?? "-"}`,
    `provider: ${context.provider}`,
    `model: ${context.model}`,
    `effort: ${context.effort}`,
  ]
  const variables = Object.entries(context.variables)
  if (variables.length === 0) {
    lines.push("vars: -")
  } else {
    lines.push("vars:")
    for (const [name, value] of variables) {
      lines.push(`  $${name} = ${value}`)
    }
  }
  return lines.join("\n")
}

function formatContextHomeKernel(session: RuntimeSession | null): string {
  const homeKernel = session?.host_daemon_id?.trim() || ""
  const homeMachine = session?.host_machine_id?.trim() || ""
  if (homeKernel && homeMachine) {
    return `${homeKernel}@${homeMachine}`
  }
  return homeKernel || homeMachine || "-"
}

function formatContextRuntimeAuthority(session: RuntimeSession | null): string {
  if (!session) {
    return "unknown until session state is available"
  }
  return "home owns sessions, prompts, grants, and live sync; workers execute leases and projected tools"
}

function findCurrentAgent(context: ShellContext, session: RuntimeSession | null): AgentInstance | null {
  if (!context.agentId) {
    return null
  }
  return session?.agents.find((agent) => (
    agent.id === context.agentId || agent.agent_ref === context.agentId || agent.alias === context.agentId
  )) ?? null
}

function formatContextAgentPlacement(
  agent: AgentInstance,
  slices: readonly SliceRecord[],
  sliceLookupError: string | null,
): string {
  const remote = agent.remote_execution
  if (!remote) {
    return "worker-local"
  }
  const slice = sliceForRemoteAgent(agent, slices)
  const placement = slice ? `slice ${slice.name || slice.id}` : "remote"
  const worker = remote.worker_machine_id || remote.worker_kernel_id
  const parts = [
    worker ? `worker=${worker}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `active_run=${remote.active_worker_provider_run_id}` : null,
    sliceLookupError ? `slice_lookup=${sliceLookupError}` : null,
  ].filter(Boolean)
  return `${placement}${parts.length > 0 ? ` (${parts.join(", ")})` : ""}`
}

function formatContextProviderRun(
  agent: AgentInstance,
  session: RuntimeSession | null,
  activeProviderRun: RuntimeProviderRun | null,
  providerRunLookupError: string | null,
): string {
  const sessionRunId = session?.active_provider_run_id ?? null
  const sessionRunAgentId = activeProviderRun?.agent_instance_id ?? null
  const workerRunId = agent.remote_execution?.active_worker_provider_run_id ?? null
  if (sessionRunId && sessionRunAgentId && sessionRunAgentId !== agent.id) {
    return [
      `session=${sessionRunId} owned_by=${sessionRunAgentId}`,
      workerRunId ? `worker=${workerRunId}` : null,
    ].filter(Boolean).join(", ")
  }
  if (sessionRunId && !sessionRunAgentId) {
    return [
      `session=${sessionRunId} owner unknown${providerRunLookupError ? `; ${providerRunLookupError}` : ""}`,
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

function formatContextProviderRunNextAction(
  agent: AgentInstance,
  session: RuntimeSession | null,
  activeProviderRun: RuntimeProviderRun | null,
): string[] {
  return providerRunRecoveryActions({
    agent,
    activeProviderRunId: session?.active_provider_run_id,
    activeProviderRunAgentId: activeProviderRun?.agent_instance_id,
  }).map((action) => `provider run next: ${action}`)
}

function formatContextExtensionSummary(agent: AgentInstance): string {
  const grants = agent.extension_grants ?? []
  if (grants.length === 0) {
    return agent.remote_extension_manifest_sync?.pending_revoke ? "none (final revoke pending)" : "none"
  }
  return formatExtensionGrantPlacementSummary(grants, {
    remote: Boolean(agent.remote_execution),
    countSeparator: "=",
  })
}

function formatContextRemoteExtensionSyncLines(agent: AgentInstance): string[] {
  if (!agent.remote_execution) {
    return ["remote extension sync: not applicable (worker-local agent; no home-proxy manifest)"]
  }
  const sync = agent.remote_extension_manifest_sync
  if (!sync && !hasActiveHomeProxyExtensionGrants(agent.extension_grants)) {
    return ["remote extension sync: not applicable (no active home-proxy tools)"]
  }
  const lines = [`remote extension sync: ${formatRemoteExtensionSyncStatusLine(sync, {
    includeHash: true,
    includeNext: false,
    agentRef: agent.agent_ref,
    workerMachineId: agent.remote_execution.worker_machine_id,
    errorPrefix: "error=",
  })}`]
  const next = remoteExtensionSyncNextAction(
    sync,
    agent.agent_ref,
    agent.remote_execution.worker_machine_id,
  )
  if (next) {
    lines.push(`remote extension next: ${next}`)
  }
  return lines
}

function sliceForRemoteAgent(agent: AgentInstance, slices: readonly SliceRecord[]): SliceRecord | null {
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

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
