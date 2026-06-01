import type {
  RuntimeSession,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import {
  parseWorkspaceLiveSyncModeCommand,
  workspaceLiveSyncModeProtocolValue,
  type WorkspaceLiveSyncModeProtocolValue,
} from "@arroba/kernel-client/workspace-live-sync-mode"
import {
  prepareLocalGitWorktree,
  suggestNamedWorktreePath,
  worktreeAliasConfigPath,
  type LocalGitWorktreeOptions,
} from "./command-worktree-placement.js"
import { resolve as resolvePath } from "node:path"

type FooterTone = "info" | "error"

type WorkspaceLinkPayload = {
  link: WorkspaceLinkDefinition
  session?: RuntimeSession
}

export type WorkspaceCommandHandlerDeps = {
  currentWorkspaceTarget: () => string
  currentWorktreeTarget: () => string
  setWorkspaceTarget: (workspace: string) => void
  setWorktreeTarget: (worktree: string) => void
  baseWorktree: string
  hasDynamicWorktreeTarget: boolean
  isAttached: () => boolean
  sessionState: () => Pick<RuntimeSession, "id">
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  applySessionState: (session: RuntimeSession) => void
  prepareLocalGitWorktree?: (options: LocalGitWorktreeOptions) => Promise<string>
  createWorkspaceLink?: (name: string) => Promise<WorkspaceLinkPayload>
  listWorkspaceLinks?: () => Promise<WorkspaceLinkDefinition[]>
  showWorkspaceLink?: (linkRef: string) => Promise<WorkspaceLinkDefinition>
  attachWorkspaceLink?: (linkRef: string, repoRoot?: string | null) => Promise<WorkspaceLinkPayload>
  detachWorkspaceLink?: (linkRef: string, repoRoot?: string | null) => Promise<WorkspaceLinkPayload & { detached: unknown[] }>
  getWorkspaceLiveSyncStatus?: () => Promise<WorkspaceLiveSyncStatus>
  setWorkspaceLiveSyncStatus?: (status: WorkspaceLiveSyncStatus | null) => void
  setWorkspaceLiveSyncMode?: (sessionId: string, mode: WorkspaceLiveSyncModeProtocolValue) => Promise<unknown>
  setUserConfigValue?: (path: string, value: string) => Promise<unknown>
  unsetUserConfigValue?: (path: string) => Promise<unknown>
}

export async function handleWorkspaceSlashCommand(
  deps: WorkspaceCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "workspace" }>,
): Promise<void> {
  const [resource, action, ...args] = command.args
  if (resource === "sync") {
    await handleWorkspaceSyncCommand(deps, action, args)
    return
  }
  if (resource && resource !== "link") {
    setWorkspaceTarget(deps, [resource, action, ...args].filter(Boolean).join(" "))
    return
  }
  if (resource !== "link") {
    deps.flashFooter(`workspace target: ${deps.currentWorkspaceTarget()}`, "info")
    return
  }
  await handleWorkspaceLinkCommand(deps, action, args)
}

async function handleWorkspaceSyncCommand(
  deps: WorkspaceCommandHandlerDeps,
  action: string | undefined,
  args: string[],
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before viewing workspace live sync", "error")
    return
  }
  if (!action || action === "status") {
    if (!deps.getWorkspaceLiveSyncStatus) {
      deps.flashFooter("workspace live sync status is not available", "error")
      return
    }
    const status = await deps.getWorkspaceLiveSyncStatus()
    deps.setWorkspaceLiveSyncStatus?.(status)
    deps.appendNotice(formatWorkspaceLiveSyncStatus(status))
    deps.flashFooter(`workspace live sync ${status.footer_state}`, "info")
    return
  }
  if (action === "targets") {
    if (!deps.getWorkspaceLiveSyncStatus) {
      deps.flashFooter("workspace live sync status is not available", "error")
      return
    }
    const status = await deps.getWorkspaceLiveSyncStatus()
    deps.setWorkspaceLiveSyncStatus?.(status)
    deps.appendNotice(formatWorkspaceLiveSyncTargets(status))
    deps.flashFooter(`workspace live sync targets: ${status.targets.length}`, "info")
    return
  }
  if (action === "conflicts") {
    if (!deps.getWorkspaceLiveSyncStatus) {
      deps.flashFooter("workspace live sync status is not available", "error")
      return
    }
    const status = await deps.getWorkspaceLiveSyncStatus()
    deps.setWorkspaceLiveSyncStatus?.(status)
    deps.appendNotice(formatWorkspaceLiveSyncConflicts(status))
    deps.flashFooter(`workspace live sync conflicts: ${status.conflicts.length}`, "info")
    return
  }
  if (action === "ignore") {
    if (!deps.getWorkspaceLiveSyncStatus) {
      deps.flashFooter("workspace live sync status is not available", "error")
      return
    }
    const status = await deps.getWorkspaceLiveSyncStatus()
    deps.setWorkspaceLiveSyncStatus?.(status)
    deps.appendNotice([
      `Ignore file: ${status.ignore.ignore_file ?? "none"}`,
      ...status.ignore.rules.map((pattern) => `rule ${pattern}`),
      ...status.ignore.force_excludes.map((pattern) => `force-exclude ${pattern}`),
    ].join("\n"))
    deps.flashFooter("workspace live sync ignore rules", "info")
    return
  }
  if (action === "enable") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "managed")
    if (!mode || mode === "off" || args.length > 1 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync enable [managed|tracked]", "error")
      return
    }
    await deps.setWorkspaceLiveSyncMode(deps.sessionState().id, mode)
    deps.flashFooter(`current session workspace live sync enabled: ${mode}`, "info")
    return
  }
  if (action === "off" || action === "managed" || action === "tracked") {
    if (args.length > 0 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync off|managed|tracked", "error")
      return
    }
    await deps.setWorkspaceLiveSyncMode(deps.sessionState().id, workspaceLiveSyncModeProtocolValue(action))
    deps.flashFooter(action === "off" ? "current session workspace live sync disabled" : `current session workspace live sync set to ${action}`, "info")
    return
  }
  if (action === "disable") {
    if (args.length > 0 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync disable", "error")
      return
    }
    await deps.setWorkspaceLiveSyncMode(deps.sessionState().id, "unrestricted")
    deps.flashFooter("current session workspace live sync disabled", "info")
    return
  }
  if (action === "mode") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "")
    if (!mode || args.length !== 1 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync mode off|managed|tracked", "error")
      return
    }
    await deps.setWorkspaceLiveSyncMode(deps.sessionState().id, workspaceLiveSyncModeProtocolValue(mode))
    deps.flashFooter(mode === "off" ? "current session workspace live sync disabled" : `current session workspace live sync set to ${mode}`, "info")
    return
  }
  if (action === "link") {
    await attachWorkspaceLink(deps, args[0], args[1], {
      usage: "/workspace sync link <name-or-id> [repo-root]",
      success: (repoRoot, link) => `linked ${repoRoot} for workspace live sync via ${link.name}; recommended mode: managed`,
    })
    return
  }
  deps.flashFooter("usage: /workspace sync status|targets|conflicts|ignore|off|managed|tracked|link", "error")
}

export async function handleWorktreeSlashCommand(
  deps: WorkspaceCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "worktree" }>,
): Promise<void> {
  const [action, ...args] = command.args
  if (!action) {
    deps.flashFooter(`worktree target: ${deps.currentWorktreeTarget()}`, "info")
    return
  }
  if (action === "name") {
    await setWorktreeAlias(deps, args.join(" ").trim())
    return
  }
  if (action === "create" || action === "new") {
    await createWorktreeTarget(deps, args)
    return
  }
  const worktreePath = resolvePath(deps.currentWorkspaceTarget(), [action, ...args].join(" "))
  deps.setWorktreeTarget(worktreePath)
  deps.flashFooter(`next-session worktree set to ${worktreePath}`, "info")
}

function setWorkspaceTarget(
  deps: WorkspaceCommandHandlerDeps,
  path: string,
): void {
  const previousWorktreeTarget = deps.currentWorktreeTarget()
  const previousWorkspaceTarget = deps.currentWorkspaceTarget()
  const workspacePath = resolvePath(deps.currentWorktreeTarget(), path)
  deps.setWorkspaceTarget(workspacePath)
  if (
    !deps.hasDynamicWorktreeTarget
    || previousWorktreeTarget === deps.baseWorktree
    || previousWorktreeTarget === previousWorkspaceTarget
  ) {
    deps.setWorktreeTarget(workspacePath)
  }
  deps.flashFooter(`next-session workspace set to ${workspacePath}`, "info")
}

async function handleWorkspaceLinkCommand(
  deps: WorkspaceCommandHandlerDeps,
  action: string | undefined,
  args: string[],
): Promise<void> {
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before managing workspace links", "error")
    return
  }
  if (action === "create" || action === "new") {
    await createWorkspaceLink(deps, args[0])
    return
  }
  if (!action || action === "list" || action === "ls") {
    await listWorkspaceLinks(deps)
    return
  }
  if (action === "show") {
    await showWorkspaceLink(deps, args[0])
    return
  }
  if (action === "attach") {
    await attachWorkspaceLink(deps, args[0], args[1])
    return
  }
  if (action === "detach") {
    await detachWorkspaceLink(deps, args[0], args[1])
    return
  }
  deps.flashFooter("usage: /workspace link create|list|show|attach|detach", "error")
}

async function createWorkspaceLink(
  deps: WorkspaceCommandHandlerDeps,
  name: string | undefined,
): Promise<void> {
  if (!name || !deps.createWorkspaceLink) {
    deps.flashFooter("usage: /workspace link create <name>", "error")
    return
  }
  const payload = await deps.createWorkspaceLink(name)
  if (payload.session) deps.applySessionState(payload.session)
  deps.flashFooter(`created workspace link ${payload.link.name}`, "info")
}

async function listWorkspaceLinks(deps: WorkspaceCommandHandlerDeps): Promise<void> {
  if (!deps.listWorkspaceLinks) {
    deps.flashFooter("workspace links are not available", "error")
    return
  }
  const links = await deps.listWorkspaceLinks()
  deps.appendNotice(formatWorkspaceLinks(links))
  deps.flashFooter(`listed ${links.length} workspace link${links.length === 1 ? "" : "s"}`, "info")
}

function formatWorkspaceLinks(links: WorkspaceLinkDefinition[]): string {
  if (links.length === 0) {
    return "No workspace links in this session."
  }
  return links.map((link) => (
    `${link.name} (${link.link_id}) attachments=${link.attachments?.length ?? 0}`
  )).join("\n")
}

async function showWorkspaceLink(
  deps: WorkspaceCommandHandlerDeps,
  linkRef: string | undefined,
): Promise<void> {
  if (!linkRef || !deps.showWorkspaceLink) {
    deps.flashFooter("usage: /workspace link show <name-or-id>", "error")
    return
  }
  const link = await deps.showWorkspaceLink(linkRef)
  deps.appendNotice(formatWorkspaceLinkDetails(link))
  deps.flashFooter(`showing workspace link ${link.name}`, "info")
}

function formatWorkspaceLinkDetails(link: WorkspaceLinkDefinition): string {
  const lines = [
    `Workspace link ${link.name} (${link.link_id})`,
    `created_by=${link.created_by_user_id}`,
    `attachments=${link.attachments?.length ?? 0}`,
  ]
  for (const attachment of link.attachments ?? []) {
    const branch = attachment.branch ? ` branch=${attachment.branch}` : ""
    lines.push(`- ${attachment.user_id} ${attachment.repo_root}${branch}`)
  }
  return lines.join("\n")
}

function formatWorkspaceLiveSyncStatus(status: WorkspaceLiveSyncStatus): string {
  const next = workspaceLiveSyncNextAction(status)
  return [
    `Workspace live sync: ${status.mode} footer=${status.footer_state}`,
    "Scope: selected workspace/worktree only; other repositories are unrestricted",
    `Sync groups: ${status.sync_groups.length}`,
    `Targets: ${status.targets.length}`,
    `Conflicts: ${status.conflicts.length}`,
    `Ignore: ${status.ignore.ignore_file ?? "none"}`,
    next ? `Next: ${next}` : "",
    status.ignore.rules.length > 0 ? `Rules: ${status.ignore.rules.join(", ")}` : "",
    status.ignore.force_excludes.length > 0 ? `Force excludes: ${status.ignore.force_excludes.join(", ")}` : "",
    formatWorkspaceLiveSyncTargets(status),
    formatWorkspaceLiveSyncConflicts(status),
  ].filter(Boolean).join("\n")
}

function formatWorkspaceLiveSyncTargets(status: WorkspaceLiveSyncStatus): string {
  const groups = status.sync_groups.map((group) => (
    `Group ${group.group_name} (${group.group_id}) targets=${group.target_count} ready=${group.ready_targets} degraded=${group.degraded_targets} conflicts=${group.conflicted_targets}`
  ))
  const targets = status.targets.map((target) => {
    const branch = target.branch ? ` branch=${target.branch}` : ""
    return `- ${target.status} ${target.link_name}: ${target.user_id} ${target.repo_root}${branch}`
  })
  const lines = groups.concat(targets)
  if (lines.length === 0) {
    return "No workspace live sync targets.\nNext: link another repo or worktree with /workspace sync link <name-or-id> [repo-root]"
  }
  const next = workspaceLiveSyncNextAction(status)
  return next ? `${lines.join("\n")}\nNext: ${next}` : lines.join("\n")
}

function formatWorkspaceLiveSyncConflicts(status: WorkspaceLiveSyncStatus): string {
  if (status.conflicts.length === 0) {
    return "No workspace live sync conflicts."
  }
  return status.conflicts.map((conflict) => (
    `- ${conflict.path} source=${conflict.source_agent_id} target=${conflict.target_user_id}:${conflict.target_repo_root} next=${conflict.next_action}`
  )).join("\n")
}

function workspaceLiveSyncNextAction(status: WorkspaceLiveSyncStatus): string {
  if (status.conflicts.length > 0 || status.footer_state === "conflict") {
    return "inspect /workspace sync conflicts, ask an agent to reconcile, then rerun /workspace sync status"
  }
  if (status.mode === "unrestricted" || status.footer_state === "off") {
    return "enable with /workspace sync managed, or use /workspace sync tracked for turn-end fanout"
  }
  if (status.targets.length === 0) {
    return "link another repo or worktree with /workspace sync link <name-or-id> [repo-root]"
  }
  if (status.sync_groups.some((group) => group.degraded_targets > 0) || status.targets.some((target) => target.status === "degraded")) {
    return "inspect /workspace sync targets and reconnect or repair degraded target kernels"
  }
  if (status.footer_state === "syncing") {
    return "wait for sync to settle, or inspect /workspace sync targets"
  }
  return ""
}

async function attachWorkspaceLink(
  deps: WorkspaceCommandHandlerDeps,
  linkRef: string | undefined,
  repoRootArg: string | undefined,
  messages?: {
    usage?: string
    success?: (repoRoot: string, link: WorkspaceLinkDefinition) => string
  },
): Promise<void> {
  const repoRoot = repoRootArg ? resolvePath(deps.currentWorktreeTarget(), repoRootArg) : deps.currentWorktreeTarget()
  if (!linkRef || !deps.attachWorkspaceLink) {
    deps.flashFooter(`usage: ${messages?.usage ?? "/workspace link attach <name-or-id> [repo-root]"}`, "error")
    return
  }
  const payload = await deps.attachWorkspaceLink(linkRef, repoRoot)
  if (payload.session) deps.applySessionState(payload.session)
  deps.flashFooter(
    messages?.success?.(repoRoot, payload.link)
      ?? `attached ${repoRoot} to workspace link ${payload.link.name}; enable with /workspace sync managed (recommended)`,
    "info",
  )
}

async function detachWorkspaceLink(
  deps: WorkspaceCommandHandlerDeps,
  linkRef: string | undefined,
  repoRootArg: string | undefined,
): Promise<void> {
  const repoRoot = repoRootArg ? resolvePath(deps.currentWorktreeTarget(), repoRootArg) : deps.currentWorktreeTarget()
  if (!linkRef || !deps.detachWorkspaceLink) {
    deps.flashFooter("usage: /workspace link detach <name-or-id> [repo-root]", "error")
    return
  }
  const payload = await deps.detachWorkspaceLink(linkRef, repoRoot)
  if (payload.session) deps.applySessionState(payload.session)
  deps.flashFooter(`detached ${payload.detached.length} workspace link attachment${payload.detached.length === 1 ? "" : "s"} from ${payload.link.name}`, "info")
}

async function setWorktreeAlias(
  deps: WorkspaceCommandHandlerDeps,
  alias: string,
): Promise<void> {
  if (!deps.setUserConfigValue || !deps.unsetUserConfigValue) {
    deps.flashFooter("worktree naming is unavailable in this build", "error")
    return
  }
  const configPath = worktreeAliasConfigPath(deps.currentWorktreeTarget())
  if (!alias) {
    await deps.unsetUserConfigValue(configPath)
    deps.flashFooter(`cleared worktree name for ${deps.currentWorktreeTarget()}`, "info")
    return
  }
  await deps.setUserConfigValue(configPath, alias)
  deps.flashFooter(`named ${deps.currentWorktreeTarget()} as ${alias}`, "info")
}

async function createWorktreeTarget(
  deps: WorkspaceCommandHandlerDeps,
  args: string[],
): Promise<void> {
  const [branch, explicitPath, ...rest] = args
  if (!branch) {
    deps.flashFooter("usage: /worktree create <branch> [directory] [--from <ref>]", "error")
    return
  }
  let fromRef: string | undefined
  for (let index = 0; index < rest.length; index += 1) {
    if (rest[index] === "--from") {
      fromRef = rest[index + 1]
    }
  }
  const targetDirectory = suggestNamedWorktreePath(deps.currentWorkspaceTarget(), branch, explicitPath)
  const createdPath = await prepareLocalGitWorktree({
    baseDirectory: deps.currentWorkspaceTarget(),
    targetDirectory,
    branch,
    fromRef,
  }, deps.prepareLocalGitWorktree)
  deps.setWorktreeTarget(createdPath)
  deps.flashFooter(`next-session worktree set to ${createdPath}`, "info")
}
