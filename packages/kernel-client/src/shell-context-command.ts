import type { AgentInstance, RuntimeProviderRun, RuntimeSession, SliceRecord } from "./kernel-types.js"
import { getProviderRunRequest, getSessionStateRequest, listSlicesRequest } from "./ipc-requests.js"
import type { ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  parseExecutionMode,
  parsePermissionLevel,
} from "./shell-agent-policy.js"

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
  const currentAgentBusy = currentAgentId
    ? Boolean(session?.prompt_states?.[currentAgentId]?.active_prompt)
      || Boolean(session?.prompt_states?.[currentAgentId]?.queued_prompts?.length)
      || Boolean(session?.active_prompt?.target_agent_id === currentAgentId)
      || Boolean(session?.queued_prompts?.some((prompt) => prompt.target_agent_id === currentAgentId))
      || Boolean(currentAgent?.is_processing)
    : false
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
    `home kernel: ${session?.host_daemon_id?.trim() || session?.host_machine_id?.trim() || "-"}`,
    `workspace live sync: ${formatContextWorkspaceLiveSyncMode(session)}`,
    `attachment: ${context.attachmentId ?? "-"}`,
    `agent: ${agentLabel}`,
    ...(currentAgent ? [
      `agent placement: ${formatContextAgentPlacement(currentAgent, slices, sliceLookupError)}`,
      `provider run: ${formatContextProviderRun(currentAgent, session, activeProviderRun, providerRunLookupError)}`,
      `extensions: ${formatContextExtensionSummary(currentAgent)}`,
      `remote extension sync: ${formatContextRemoteExtensionSync(currentAgent)}`,
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

function findCurrentAgent(context: ShellContext, session: RuntimeSession | null): AgentInstance | null {
  if (!context.agentId) {
    return null
  }
  return session?.agents.find((agent) => (
    agent.id === context.agentId || agent.agent_ref === context.agentId || agent.alias === context.agentId
  )) ?? null
}

function formatContextWorkspaceLiveSyncMode(session: RuntimeSession | null): string {
  const mode = session?.workspace_live_sync_mode
  if (!mode) {
    return "config default"
  }
  if (mode === "unrestricted") {
    return "off"
  }
  return `${mode} (selected workspace/worktree only; other repositories unrestricted)`
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

function formatContextExtensionSummary(agent: AgentInstance): string {
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
  const placement = agent.remote_execution ? "home-proxy/passive-snapshot" : "worker-local"
  return `${grants.length} grant${grants.length === 1 ? "" : "s"} (${placement}${byKind ? `; ${byKind}` : ""})`
}

function formatContextRemoteExtensionSync(agent: AgentInstance): string {
  if (!agent.remote_execution) {
    return "not applicable"
  }
  const worker = agent.remote_execution.worker_machine_id
    ? `; run /machine kernels ${agent.remote_execution.worker_machine_id}`
    : ""
  const action = `run /extension sync-status ${agent.agent_ref}${worker}; use /extension sync-retry ${agent.agent_ref} after worker connectivity is healthy`
  const sync = agent.remote_extension_manifest_sync
  if (!sync) {
    return `pending; next=${action}`
  }
  const details = [
    sync.state,
    sync.pending_revoke ? "pending revoke" : null,
    sync.manifest_hash ? `hash=${sync.manifest_hash.slice(0, 12)}` : null,
    sync.last_error ? `error=${sync.last_error}` : null,
  ].filter(Boolean)
  const needsAction = sync.state === "failed" || sync.state === "stale" || sync.pending_revoke || sync.last_error
  if (needsAction) {
    details.push(`next=${action}`)
  }
  return details.join(", ")
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

function expectSessionState(response: Record<string, unknown>): RuntimeSession {
  if ("SessionState" in response) {
    return (response.SessionState as { session: RuntimeSession }).session
  }
  return expectVariant<{ session: RuntimeSession }>(response, "SessionStateLoaded").session
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
