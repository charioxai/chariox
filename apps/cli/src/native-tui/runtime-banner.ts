import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "../cli-types.js"
import { formatExtensionGrantPlacement } from "@arroba/kernel-client/extension-grant-placement"
import { formatWorkspaceLiveSyncModeLabel } from "@arroba/kernel-client/workspace-live-sync-mode"

export type NativeTuiRuntimeBannerInput = {
  readonly surface: string
  readonly session: RuntimeSession
  readonly agent: AgentInstance
  readonly worktree: string
  readonly run?: RuntimeProviderRun | null
  readonly grantedMcps?: readonly string[]
  readonly grantedSkills?: readonly string[]
  readonly providerLines?: readonly string[]
  readonly promptPolicy?: string
}

export function formatNativeTuiRuntimeBanner(input: NativeTuiRuntimeBannerInput): string {
  return [
    `[arroba ${input.surface}]`,
    `  arroba session: ${formatSession(input.session)}`,
    `  arroba agent:   ${formatAgent(input.agent)}`,
    `  home kernel:    ${formatHomeKernel(input.session)}`,
    `  worktree:       ${input.worktree || input.agent.worktree_id || input.session.worktree_id || "-"}`,
    `  placement:      ${formatAgentPlacement(input.agent)}`,
    `  live sync:      ${formatWorkspaceLiveSyncModeLabel(input.session.workspace_live_sync_mode)}`,
    `  extensions:     ${formatGrantedExtensions(input.agent, input.grantedMcps ?? [], input.grantedSkills ?? [])}`,
    ...(input.run ? [`  provider run:   ${input.run.id}`] : []),
    ...(input.providerLines ?? []),
    ...(input.promptPolicy ? [`  prompt policy:  ${input.promptPolicy}`] : []),
    "",
  ].join("\n")
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

function formatSession(session: RuntimeSession): string {
  return `${session.id}${session.alias ? ` (${session.alias})` : ""}`
}

function formatAgent(agent: AgentInstance): string {
  return `${agent.id}${agent.alias ? ` (${agent.alias})` : ""}`
}

function formatHomeKernel(session: RuntimeSession): string {
  const daemon = session.host_daemon_id?.trim()
  const machine = session.host_machine_id?.trim()
  if (daemon && machine) return `${daemon}@${machine}`
  return daemon || machine || "-"
}

function formatAgentPlacement(agent: AgentInstance): string {
  const remote = agent.remote_execution
  if (!remote) return "worker-local"
  const parts = [
    remote.worker_machine_id ? `worker=${remote.worker_machine_id}` : null,
    remote.worker_kernel_id ? `kernel=${remote.worker_kernel_id}` : null,
    remote.execution_lease_id ? `lease=${remote.execution_lease_id}` : null,
    remote.leased_agent_id ? `leased_agent=${remote.leased_agent_id}` : null,
    remote.active_worker_provider_run_id ? `active_run=${remote.active_worker_provider_run_id}` : null,
  ].filter(Boolean)
  return `remote${parts.length > 0 ? ` (${parts.join(", ")})` : ""}`
}
