import { resolve as resolvePath } from "node:path"

import type {
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
  showWorkspaceLinkRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  formatWorkspaceLiveSyncStatus,
  formatWorkspaceLinkDetails,
  formatWorkspaceLinks,
} from "./shell-workspace-format.js"

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
    return { ok: false, message: "usage: workspace sync status|targets|conflicts|ignore|mode|enable|disable|link or workspace link create|list|show|attach|detach" }
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
        message: `attached ${repoRoot} to workspace link ${payload.link.name}; enroll with \`workspace sync enable managed\` (recommended)`,
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
  const sessionId = context.sessionId
  if (!sessionId) {
    return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
  }
  if (!action || action === "status") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return { ok: true, message: formatWorkspaceLiveSyncStatus(payload.status), data: payload }
  }
  if (action === "targets") {
    const response = await deps.client.send(getWorkspaceLiveSyncStatusRequest(sessionId))
    const payload = expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus")
    return {
      ok: true,
      message: payload.status.targets.length === 0
        ? "no workspace live sync targets"
        : payload.status.targets.map((target) => {
          const branch = target.branch ? ` branch=${target.branch}` : ""
          return `${target.status} ${target.link_name}: ${target.user_id} ${target.repo_root}${branch}`
        }).join("\n"),
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
        ...payload.status.ignore.force_excludes.map((pattern) => `- ${pattern}`),
      ].join("\n"),
      data: payload,
    }
  }
  if (action === "enable") {
    const mode = normalizeWorkspaceLiveSyncMode(args[0] ?? "managed")
    if (!mode || args.length > 1) {
      return { ok: false, message: "usage: workspace sync enable [managed|tracked]" }
    }
    if (mode === "unrestricted") {
      return { ok: false, message: "usage: workspace sync enable [managed|tracked]" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest(mode))
    return { ok: true, message: `workspace live sync enabled: ${mode}`, data: response }
  }
  if (action === "disable") {
    if (args.length > 0) {
      return { ok: false, message: "usage: workspace sync disable" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest("unrestricted"))
    return { ok: true, message: "workspace live sync disabled", data: response }
  }
  if (action === "mode") {
    const mode = normalizeWorkspaceLiveSyncMode(args[0] ?? "")
    if (!mode || args.length !== 1) {
      return { ok: false, message: "usage: workspace sync mode managed|tracked|unrestricted" }
    }
    const response = await deps.client.send(setWorkspaceLiveSyncModeRequest(mode))
    return { ok: true, message: `workspace live sync mode set to ${mode}`, data: response }
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
      message: `linked ${repoRoot} for workspace live sync via ${payload.link.name}; recommended mode: managed`,
      data: payload,
      contextUpdates: { sessionId: payload.session.id },
    }
  }
  if (args.length > 0) {
    return { ok: false, message: "usage: workspace sync status|targets|conflicts|ignore|enable|disable|mode|link" }
  }
  return { ok: false, message: "usage: workspace sync status|targets|conflicts|ignore|enable|disable|mode|link" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}

function normalizeWorkspaceLiveSyncMode(value: string): "managed" | "tracked" | "unrestricted" | null {
  if (value === "managed" || value === "tracked" || value === "unrestricted") return value
  return null
}
