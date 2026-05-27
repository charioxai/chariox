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
    return { ok: false, message: "usage: workspace sync status|targets|conflicts|ignore|mode|enable|disable or workspace link create|list|show|attach|detach" }
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
        message: `attached ${repoRoot} to workspace link ${payload.link.name}`,
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
        : payload.status.targets.map((target) => `${target.status} ${target.link_name}: ${target.repo_root}`).join("\n"),
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
        : payload.status.conflicts.map((conflict) => `${conflict.path}: ${conflict.next_action}`).join("\n"),
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
        ...payload.status.ignore.force_excludes.map((pattern) => `- ${pattern}`),
      ].join("\n"),
      data: payload,
    }
  }
  if (["mode", "enable", "disable", "link"].includes(action)) {
    return { ok: false, message: `workspace sync ${action} is not wired in arroba-shell yet; use config workspace-live-sync and workspace link commands` }
  }
  if (args.length > 0) {
    return { ok: false, message: "usage: workspace sync status|targets|conflicts|ignore" }
  }
  return { ok: false, message: "usage: workspace sync status|targets|conflicts|ignore" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
