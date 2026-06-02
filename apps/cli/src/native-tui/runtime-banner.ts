import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
} from "../cli-types.js"
import {
  formatExtensionGrantPlacement,
  formatExtensionGrantRuntimeDetail,
} from "@arroba/kernel-client/extension-grant-placement"
import { formatRemoteExtensionSyncStatusLine, remoteExtensionSyncNextAction } from "@arroba/kernel-client/shell-capability-format"
import { formatSliceProviderAccounts, formatSliceScope } from "../slice-format.js"
import { formatWorkspaceLiveSyncModeLabel } from "@arroba/kernel-client/workspace-live-sync-mode"

export type NativeTuiRuntimeBannerInput = {
  readonly surface: string
  readonly session: RuntimeSession
  readonly agent: AgentInstance
  readonly worktree: string
  readonly slices?: readonly SliceRecord[]
  readonly sliceLookupError?: string | null
  readonly run?: RuntimeProviderRun | null
  readonly grantedMcps?: readonly string[]
  readonly grantedSkills?: readonly string[]
  readonly providerLines?: readonly string[]
  readonly promptPolicy?: string
}

export function formatNativeTuiRuntimeBanner(input: NativeTuiRuntimeBannerInput): string {
  const slice = sliceForRemoteAgent(input.agent, input.slices ?? [])
  return [
    `[arroba ${input.surface}]`,
    `  arroba session: ${formatSession(input.session)}`,
    `  arroba agent:   ${formatAgent(input.agent)}`,
    `  home kernel:    ${formatHomeKernel(input.session)}`,
    `  worktree:       ${input.worktree || input.agent.worktree_id || input.session.worktree_id || "-"}`,
    `  placement:      ${formatAgentPlacement(input.agent, slice)}`,
    ...formatSliceLines(slice, input.agent, input.sliceLookupError),
    `  live sync:      ${formatWorkspaceLiveSyncModeLabel(input.session.workspace_live_sync_mode)}`,
    `  extensions:     ${formatGrantedExtensions(input.agent, input.grantedMcps ?? [], input.grantedSkills ?? [])}`,
    `  ext runtime:    ${formatExtensionGrantRuntimeDetail(nativeBannerExtensionGrants(input.agent, input.grantedMcps ?? [], input.grantedSkills ?? []), Boolean(input.agent.remote_execution))}`,
    ...formatRemoteExtensionSync(input.agent, input.grantedMcps ?? []),
    ...(input.run ? [`  provider run:   ${input.run.id}`] : []),
    ...(input.providerLines ?? []),
    ...(input.promptPolicy ? [`  prompt policy:  ${input.promptPolicy}`] : []),
    "",
  ].join("\n")
}

function nativeBannerExtensionGrants(
  agent: AgentInstance,
  mcps: readonly string[],
  skills: readonly string[],
): readonly { readonly kind: string }[] {
  const grants = [...(agent.extension_grants ?? [])]
  if (mcps.length > 0 && !grants.some((grant) => grant.kind === "mcp")) {
    grants.push({ kind: "mcp", name: "__native_banner_mcp__" })
  }
  if (skills.length > 0 && !grants.some((grant) => grant.kind === "skill")) {
    grants.push({ kind: "skill", name: "__native_banner_skill__" })
  }
  return grants
}

function formatGrantedExtensions(agent: AgentInstance, mcps: readonly string[], skills: readonly string[]): string {
  if (mcps.length === 0 && skills.length === 0) return "none"
  const remote = Boolean(agent.remote_execution)
  const activePlacement = formatExtensionGrantPlacement([{ kind: "mcp" }], remote)
  const skillPlacement = formatExtensionGrantPlacement([{ kind: "skill" }], remote)
  return [
    mcps.length > 0 ? `mcp=${mcps.join(",")} (${activePlacement})` : null,
    skills.length > 0 ? `skill=${skills.join(",")} (${skillPlacement})` : null,
  ].filter(Boolean).join("; ")
}

function formatRemoteExtensionSync(agent: AgentInstance, mcps: readonly string[]): string[] {
  if (!agent.remote_execution) return []
  const status = agent.remote_extension_manifest_sync
  const hasActiveHomeProxy = mcps.length > 0 || Boolean(agent.extension_grants?.some((grant) => (
    grant.kind === "mcp" || grant.kind === "script" || grant.kind === "connector"
  )))
  if (!status && !hasActiveHomeProxy) return []
  const lines = [`  remote ext sync: ${formatRemoteExtensionSyncStatusLine(status, {
    includeHash: true,
    includeNext: false,
    agentRef: agent.agent_ref,
    workerMachineId: agent.remote_execution.worker_machine_id,
    errorPrefix: "error=",
  })}`]
  const next = remoteExtensionSyncNextAction(
    status,
    agent.agent_ref,
    agent.remote_execution.worker_machine_id,
  )
  if (next) {
    lines.push(`  ext sync next:  ${next}`)
  }
  return lines
}

function formatSession(session: RuntimeSession): string {
  return `${session.id}${session.alias ? ` (${session.alias})` : ""}`
}

function formatAgent(agent: AgentInstance): string {
  const label = agent.agent_ref || agent.id
  const alias = agent.alias ? ` (${agent.alias})` : ""
  const id = agent.id && agent.id !== label ? ` [id=${agent.id}]` : ""
  return `${label}${alias}${id}`
}

function formatSliceLines(
  slice: SliceRecord | null,
  agent: AgentInstance,
  sliceLookupError: string | null | undefined,
): string[] {
  if (slice) {
    return [
      `  slice:          ${formatSliceSummary(slice)}`,
      `  slice auth:     ${formatSliceProviderAccounts(slice)}`,
    ]
  }
  if (agent.remote_execution && sliceLookupError) {
    return [`  slice lookup:   ${sliceLookupError}`]
  }
  return []
}

function formatSliceSummary(slice: SliceRecord): string {
  return `${slice.name || slice.id} (${[
    `id=${slice.id}`,
    `status=${slice.status}`,
    `display=${slice.display_mode ?? "headless"}`,
    `worktree=${formatSliceScope(slice)}`,
    `agents=${slice.agent_ids?.length ?? 0}`,
  ].join(", ")})`
}

function formatHomeKernel(session: RuntimeSession): string {
  const daemon = session.host_daemon_id?.trim()
  const machine = session.host_machine_id?.trim()
  if (daemon && machine) return `${daemon}@${machine}`
  return daemon || machine || "-"
}

function formatAgentPlacement(agent: AgentInstance, slice: SliceRecord | null): string {
  const remote = agent.remote_execution
  if (!remote) return "worker-local"
  const placement = slice ? `slice ${slice.name || slice.id}` : "remote"
  const parts = [
    remote.worker_machine_id ? `worker=${remote.worker_machine_id}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `active_run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return `${placement}${parts.length > 0 ? ` (${parts.join(", ")})` : ""}`
}

function sliceForRemoteAgent(
  agent: AgentInstance,
  slices: readonly SliceRecord[],
): SliceRecord | null {
  const remote = agent.remote_execution
  if (!remote) return null
  return slices.find((slice) => slice.agent_ids?.includes(agent.id))
    ?? slices.find((slice) =>
      slice.worker_kernel_id === remote.worker_kernel_id
      || slice.worker_kernel_ref === remote.worker_kernel_id
      || slice.worker_machine_id === remote.worker_machine_id,
    )
    ?? null
}
