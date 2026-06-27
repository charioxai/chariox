import { resolve as resolvePath } from "node:path"

import type {
  RecallEvent,
  RuntimeSession,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./kernel-types.js"
import {
  attachWorkspaceLinkRequest,
  createWorkspaceLinkRequest,
  detachWorkspaceLinkRequest,
  getWorkspaceLiveSyncStatusRequest,
  listWorkspaceLinksRequest,
  setWorkspaceLiveSyncModeRequest,
  setUserConfigValueRequest,
  showWorkspaceLinkRequest,
} from "./ipc-requests.js"
import { queryRecallRequest } from "./ipc-recall-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatWorkspaceLiveSyncAudit,
  formatWorkspaceLiveSyncDoctor,
  formatWorkspaceLiveSyncTargets,
  formatWorkspaceLiveSyncStatus,
  formatWorkspaceLinkDetails,
  formatWorkspaceLinks,
} from "./shell-workspace-format.js"
import {
  formatWorkspaceLiveSyncDefaultModeChangeMessage,
  formatWorkspaceLiveSyncModeChangeMessage,
  parseWorkspaceLiveSyncModeCommand,
  type WorkspaceLiveSyncProviderReloadSummary,
} from "./workspace-live-sync-mode.js"

type ShellWorkspaceCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeWorkspaceCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellWorkspaceCommandDeps,
): Promise<ShellCommandResult> {
  const [resource, action, ...args] = parsed.args
  if (resource === "sync") {
    return executeWorkspaceSyncCommand(action, args, context, deps)
  }
  if (resource !== "link") {
    return { ok: false, message: workspaceUsage() }
  }
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  switch (action) {
    case "create":
    case "new": {
      const name = args[0]
      if (!name) {
        return { ok: false, message: "usage: workspace link create <name>" }
      }
      const response = await deps.client.send(createWorkspaceLinkRequest(sessionId, name))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition; session: RuntimeSession }>(response, "WorkspaceLinkCreated")
      return {
        ok: true,
        message: `created workspace link ${payload.link.name} (${payload.link.link_id})`,
        data: payload,
        bindings: parsed.assignment ? { [parsed.assignment]: payload.link.link_id } : undefined,
        contextUpdates: { sessionId: payload.session.id },
      }
    }
    case undefined:
    case "list":
    case "ls": {
      const response = await deps.client.send(listWorkspaceLinksRequest(sessionId))
      const payload = expectVariant<{ links: WorkspaceLinkDefinition[] }>(response, "WorkspaceLinksListed")
      return { ok: true, message: formatWorkspaceLinks(payload.links), data: payload }
    }
    case "show": {
      const linkRef = args[0]
      if (!linkRef) {
        return { ok: false, message: "usage: workspace link show <name-or-id>" }
      }
      const response = await deps.client.send(showWorkspaceLinkRequest(sessionId, linkRef))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition }>(response, "WorkspaceLinkShown")
      return { ok: true, message: formatWorkspaceLinkDetails(payload.link), data: payload }
    }
    case "attach": {
      const linkRef = args[0]
      const repoRoot = args[1] ? resolvePath(context.worktree, args[1]) : context.worktree
      if (!linkRef) {
        return { ok: false, message: "usage: workspace link attach <name-or-id> [repo-root]" }
      }
      const response = await deps.client.send(attachWorkspaceLinkRequest(sessionId, linkRef, repoRoot))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition; session: RuntimeSession }>(response, "WorkspaceLinkAttached")
      return {
        ok: true,
        message: `attached ${repoRoot} to workspace link ${payload.link.name}; live sync mode is unchanged; choose \`workspace sync managed\` or \`workspace sync tracked\` to start syncing this session`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id },
      }
    }
    case "detach": {
      const linkRef = args[0]
      const repoRoot = args[1] ? resolvePath(context.worktree, args[1]) : context.worktree
      if (!linkRef) {
        return { ok: false, message: "usage: workspace link detach <name-or-id> [repo-root]" }
      }
      const response = await deps.client.send(detachWorkspaceLinkRequest(sessionId, linkRef, repoRoot))
      const payload = expectVariant<{ link: WorkspaceLinkDefinition; detached: unknown[]; session: RuntimeSession }>(response, "WorkspaceLinkDetached")
      return {
        ok: true,
        message: `detached ${payload.detached.length} workspace link attachment${payload.detached.length === 1 ? "" : "s"} from ${payload.link.name}`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id },
      }
    }
    default:
      return { ok: false, message: "usage: workspace link create|list|show|attach|detach" }
  }
}

async function executeWorkspaceSyncCommand(
  action: string | undefined,
  args: string[],
  context: ShellContext,
  deps: ShellWorkspaceCommandDeps,
): Promise<ShellCommandResult> {
  if (action === "default") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "")
    if (!mode || args.length !== 1) {
      return { ok: false, message: "usage: workspace sync default off|managed|tracked" }
    }
    const response = await deps.client.send(setUserConfigValueRequest("providers.workspace_live_sync", mode))
    return {
      ok: true,
      message: formatWorkspaceLiveSyncDefaultModeChangeMessage(mode),
      data: response,
    }
  }
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  if (!action || action === "status") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return { ok: true, message: formatWorkspaceLiveSyncStatus(payload.status), data: payload }
  }
  if (action === "doctor") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return { ok: true, message: formatWorkspaceLiveSyncDoctor(payload.status), data: payload }
  }
  if (action === "targets") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return {
      ok: true,
      message: formatWorkspaceLiveSyncTargets(payload.status),
      data: payload,
    }
  }
  if (action === "conflicts") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return {
      ok: true,
      message: payload.status.conflicts.length === 0
        ? "no workspace live sync conflicts"
        : payload.status.conflicts.map((conflict) => (
          `${conflict.path} source=${conflict.source_agent_id} target=${conflict.target_user_id}:${conflict.target_repo_root}: ${conflict.next_action}`
        )).join("\n"),
      data: payload,
    }
  }
  if (action === "ignore") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return {
      ok: true,
      message: [
        `ignore=${payload.status.ignore.ignore_file ?? "none"}`,
        ...payload.status.ignore.rules.map((pattern) => `rule ${pattern}`),
        ...payload.status.ignore.force_excludes.map((pattern) => `force-exclude ${pattern}`),
      ].join("\n"),
      data: payload,
    }
  }
  if (action === "audit") {
    const limit = readNumberOption(args, "--limit") ?? 20
    const response = await deps.client.send(queryRecallRequest({
      session_id: sessionId,
      kind: "workspace_live_sync_mode_changed",
      limit,
    }))
    const payload = expectVariant<{ events?: RecallEvent[] }>(response, "RecallEvents")
    return {
      ok: true,
      message: formatWorkspaceLiveSyncAudit(payload.events ?? []),
      data: payload,
    }
  }
  if (action === "enable") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "managed")
    if (!mode || args.length > 1) {
      return { ok: false, message: "usage: workspace sync enable [managed|tracked]" }
    }
    if (mode === "off") {
      return { ok: false, message: "usage: workspace sync enable [managed|tracked]" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest(sessionId, mode))
    return {
      ok: true,
      message: formatWorkspaceLiveSyncModeChangeMessage(mode, {
        action: "enabled",
        providerReload: workspaceLiveSyncProviderReloadSummary(response),
      }),
      data: response,
    }
  }
  if (action === "off" || action === "managed" || action === "tracked") {
    if (args.length > 0) {
      return { ok: false, message: "usage: workspace sync off|managed|tracked" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest(sessionId, action))
    return {
      ok: true,
      message: formatWorkspaceLiveSyncModeChangeMessage(action, {
        providerReload: workspaceLiveSyncProviderReloadSummary(response),
      }),
      data: response,
    }
  }
  if (action === "disable") {
    if (args.length > 0) {
      return { ok: false, message: "usage: workspace sync disable" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest(sessionId, "unrestricted"))
    return {
      ok: true,
      message: formatWorkspaceLiveSyncModeChangeMessage("unrestricted", {
        providerReload: workspaceLiveSyncProviderReloadSummary(response),
      }),
      data: response,
    }
  }
  if (action === "mode") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "")
    if (!mode || args.length !== 1) {
      return { ok: false, message: "usage: workspace sync mode off|managed|tracked" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest(sessionId, mode))
    return {
      ok: true,
      message: formatWorkspaceLiveSyncModeChangeMessage(mode, {
        providerReload: workspaceLiveSyncProviderReloadSummary(response),
      }),
      data: response,
    }
  }
  if (action === "link") {
    const linkRef = args[0]
    const repoRoot = args[1] ? resolvePath(context.worktree, args[1]) : context.worktree
    if (!linkRef || args.length > 2) {
      return { ok: false, message: "usage: workspace sync link <name-or-id> [repo-root]" }
    }
    const response = await deps.client.send(attachWorkspaceLinkRequest(sessionId, linkRef, repoRoot))
    const payload = expectVariant<{ link: WorkspaceLinkDefinition; session: RuntimeSession }>(response, "WorkspaceLinkAttached")
    return {
      ok: true,
      message: `linked ${repoRoot} for workspace live sync via ${payload.link.name}; live sync mode is unchanged; choose \`workspace sync managed\` or \`workspace sync tracked\` to start syncing this session`,
      data: payload,
      contextUpdates: { sessionId: payload.session.id },
    }
  }
  if (args.length > 0) {
    return { ok: false, message: workspaceSyncUsage() }
  }
  return { ok: false, message: workspaceSyncUsage() }
}

function workspaceUsage(): string {
  return `usage: ${workspaceSyncUsage()} or workspace link create|list|show|attach|detach`
}

function workspaceSyncUsage(): string {
  return "workspace sync status|doctor|targets|conflicts|ignore|audit|off|managed|tracked|default|link"
}

function readNumberOption(args: string[], flag: string): number | null {
  const index = args.indexOf(flag)
  if (index === -1) return null
  const value = args[index + 1]
  if (!value || value.startsWith("--")) return null
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function workspaceLiveSyncProviderReloadSummary(response: Record<string, unknown>): WorkspaceLiveSyncProviderReloadSummary | null {
  const payload = response.WorkspaceLiveSyncModeUpdated
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) return null
  const effects = (payload as { effects?: unknown }).effects
  if (!Array.isArray(effects)) return null
  for (const effect of effects) {
    if (!effect || typeof effect !== "object") continue
    const record = effect as { kind?: unknown, provider_reload?: unknown }
    if (record.kind !== "provider_reload") continue
    const summary = record.provider_reload
    if (!summary || typeof summary !== "object") continue
    const providerReload = summary as Record<string, unknown>
    const reloaded = numberField(providerReload, "reloaded")
    const deferred = numberField(providerReload, "deferred")
    const unaffected = numberField(providerReload, "unaffected")
    if (reloaded === null || deferred === null || unaffected === null) continue
    return { reloaded, deferred, unaffected }
  }
  return null
}

function numberField(record: Record<string, unknown>, key: string): number | null {
  const value = record[key]
  return Number.isFinite(value) ? value as number : null
}
