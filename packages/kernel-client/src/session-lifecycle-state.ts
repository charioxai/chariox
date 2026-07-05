import type { AgentInstance, RuntimeSession } from "./kernel-types.js"
import { sessionFocusedAgentId } from "./session-runtime-transition.js"

export type SessionLifecycleLaunchSelection = {
  provider: string
  model: string
  effort: string
}

export type SessionLifecycleListEntry = {
  id: string
  alias?: string | null
  workspace_id?: string
  worktree_id: string
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  host_machine_id?: string | null
  host_daemon_id?: string | null
  kernel_id?: string | null
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  status: string
  created_at_ms?: number
  last_used_at_ms?: number | null
  last_activity_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  attachment_ids?: string[]
}

export type SessionLifecycleListSession = {
  id: string
  alias?: string | null
  workspace_id?: string
  worktree_id: string
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  host_machine_id?: string | null
  host_daemon_id?: string | null
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  status: string
  created_at_ms?: number
  last_used_at_ms?: number | null
  last_activity_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  attachment_ids?: string[]
}

export type SessionLifecycleLaunchAgent = Pick<AgentInstance, "id" | "provider" | "model" | "effort">

export type SessionLifecycleLaunchSession<TAgent extends SessionLifecycleLaunchAgent = SessionLifecycleLaunchAgent> = {
  id: string
  agents: readonly TAgent[]
  focused_agent_id?: string | null
  agent_defaults?: RuntimeSession["agent_defaults"]
}

export function upsertSessionListEntry<TEntry extends { id: string }>(
  current: readonly TEntry[],
  next: TEntry,
): TEntry[] {
  const index = current.findIndex((candidate) => candidate.id === next.id)
  if (index === -1) {
    return [next, ...current]
  }
  return current.map((candidate, candidateIndex) =>
    candidateIndex === index ? { ...candidate, ...next } : candidate
  )
}

export function sessionListEntryFromSession(
  session: SessionLifecycleListSession,
): SessionLifecycleListEntry {
  const entry: SessionLifecycleListEntry = {
    id: session.id,
    worktree_id: session.worktree_id,
    status: session.status,
  }
  if (Object.prototype.hasOwnProperty.call(session, "alias")) entry.alias = session.alias ?? null
  if (Object.prototype.hasOwnProperty.call(session, "workspace_id")) {
    if (session.workspace_id !== undefined) entry.workspace_id = session.workspace_id
  }
  if (Object.prototype.hasOwnProperty.call(session, "created_at_ms")) {
    if (session.created_at_ms !== undefined) entry.created_at_ms = session.created_at_ms
  }
  if (Object.prototype.hasOwnProperty.call(session, "attachment_ids")) {
    if (session.attachment_ids !== undefined) entry.attachment_ids = session.attachment_ids
  }
  if (Object.prototype.hasOwnProperty.call(session, "workspace_live_sync_mode")) {
    entry.workspace_live_sync_mode = session.workspace_live_sync_mode ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "host_machine_id")) {
    entry.host_machine_id = session.host_machine_id ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "host_daemon_id")) {
    entry.host_daemon_id = session.host_daemon_id ?? null
    entry.kernel_id = session.host_daemon_id ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "last_used_at_ms")) {
    entry.last_used_at_ms = session.last_used_at_ms ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "last_activity_at_ms")) {
    entry.last_activity_at_ms = session.last_activity_at_ms ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "last_prompt_sent_at_ms")) {
    entry.last_prompt_sent_at_ms = session.last_prompt_sent_at_ms ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "workspace_label")) {
    entry.workspace_label = session.workspace_label ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "directory")) {
    entry.directory = session.directory ?? null
  }
  if (Object.prototype.hasOwnProperty.call(session, "worktree_label")) {
    entry.worktree_label = session.worktree_label ?? null
  }
  return entry
}

export function isCompleteSessionSnapshot(
  session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>,
): session is RuntimeSession {
  return typeof session.workspace_id === "string"
    && typeof session.worktree_id === "string"
    && typeof session.created_at_ms === "number"
    && typeof session.status === "string"
    && Array.isArray(session.attachment_ids)
    && Array.isArray(session.queued_prompts)
    && Array.isArray(session.agents)
    && Array.isArray(session.workflows)
    && Array.isArray(session.workflow_runs)
    && (Array.isArray(session.workflow_schedules) || Array.isArray(session.workflow_watchdogs))
    && Array.isArray(session.workflow_consoles)
    && typeof session.max_agents === "number"
    && typeof session.config_state === "object"
    && session.config_state !== null
}

export function resolveLaunchTargetAgent(
  session: SessionLifecycleLaunchSession<AgentInstance>,
): AgentInstance | null
export function resolveLaunchTargetAgent<TAgent extends SessionLifecycleLaunchAgent>(
  session: SessionLifecycleLaunchSession<TAgent>,
): TAgent | null
export function resolveLaunchTargetAgent<TAgent extends SessionLifecycleLaunchAgent>(
  session: SessionLifecycleLaunchSession<TAgent>,
): TAgent | null {
  const focusedAgentId = sessionFocusedAgentId(session)
  return focusedAgentId ? session.agents.find((agent) => agent.id === focusedAgentId) ?? null : null
}

export function resolveStoredAgentLaunch(
  session: SessionLifecycleLaunchSession,
  fallback: SessionLifecycleLaunchSelection,
  createdSession: boolean,
): SessionLifecycleLaunchSelection {
  if (createdSession) {
    return resolveSessionAgentDefaults(session, fallback)
  }

  const sessionDefaults = resolveSessionAgentDefaults(session, fallback)
  const focusedAgentId = sessionFocusedAgentId(session)
  const focusedAgent = focusedAgentId
    ? session.agents.find((agent) => agent.id === focusedAgentId)
    : null
  if (!focusedAgent) {
    return sessionDefaults
  }

  return {
    provider: focusedAgent.provider && focusedAgent.provider !== "default"
      ? focusedAgent.provider
      : sessionDefaults.provider,
    model: focusedAgent.model?.trim() || sessionDefaults.model,
    effort: focusedAgent.effort?.trim() || sessionDefaults.effort,
  }
}

export function resolveSessionAgentDefaults(
  session: { id: string; agent_defaults?: RuntimeSession["agent_defaults"] },
  fallback: SessionLifecycleLaunchSelection,
): SessionLifecycleLaunchSelection {
  const defaults = session.agent_defaults
  return {
    provider: defaults?.provider?.trim() && defaults.provider !== "default" ? defaults.provider : fallback.provider,
    model: defaults?.model?.trim() || fallback.model,
    effort: defaults?.effort?.trim() || fallback.effort,
  }
}
