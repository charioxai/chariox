import type {
  RuntimeSession,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import type { RecallEvent } from "@arroba/kernel-client"
import {
  formatWorkspaceLiveSyncDefaultModeChangeMessage,
  formatWorkspaceLiveSyncModeChangeMessage,
  parseWorkspaceLiveSyncModeCommand,
  workspaceLiveSyncModeProtocolValue,
  type WorkspaceLiveSyncModeProtocolValue,
  type WorkspaceLiveSyncProviderReloadSummary,
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
  listWorkspaceLiveSyncAudit?: (sessionId: string, limit?: number | null) => Promise<RecallEvent[]>
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
  if (action === "default") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "")
    if (!mode || args.length !== 1 || !deps.setUserConfigValue) {
      deps.flashFooter("usage: /workspace sync default off|managed|tracked", "error")
      return
    }
    await deps.setUserConfigValue("providers.workspace_live_sync", mode)
    deps.flashFooter(formatWorkspaceLiveSyncDefaultModeChangeMessage(mode), "info")
    return
  }
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
  if (action === "doctor") {
    if (!deps.getWorkspaceLiveSyncStatus) {
      deps.flashFooter("workspace live sync status is not available", "error")
      return
    }
    const status = await deps.getWorkspaceLiveSyncStatus()
    deps.setWorkspaceLiveSyncStatus?.(status)
    deps.appendNotice(formatWorkspaceLiveSyncDoctor(status))
    deps.flashFooter(`workspace live sync doctor: ${workspaceLiveSyncHealthLabel(status)}`, "info")
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
  if (action === "audit") {
    if (!deps.listWorkspaceLiveSyncAudit) {
      deps.flashFooter("workspace live sync audit is not available", "error")
      return
    }
    const events = await deps.listWorkspaceLiveSyncAudit(deps.sessionState().id, readNumberOption(args, "--limit"))
    deps.appendNotice(formatWorkspaceLiveSyncAudit(events))
    deps.flashFooter(`workspace live sync audit: ${events.length}`, "info")
    return
  }
  if (action === "enable") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "managed")
    if (!mode || mode === "off" || args.length > 1 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync enable [managed|tracked]", "error")
      return
    }
    const result = await applyWorkspaceLiveSyncModeChange(deps, mode)
    deps.flashFooter(formatWorkspaceLiveSyncModeChangeMessage(mode, {
      action: "enabled",
      providerReload: workspaceLiveSyncProviderReloadSummary(result),
    }), "info")
    return
  }
  if (action === "off" || action === "managed" || action === "tracked") {
    if (args.length > 0 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync off|managed|tracked", "error")
      return
    }
    const result = await applyWorkspaceLiveSyncModeChange(deps, workspaceLiveSyncModeProtocolValue(action))
    deps.flashFooter(formatWorkspaceLiveSyncModeChangeMessage(action, {
      providerReload: workspaceLiveSyncProviderReloadSummary(result),
    }), "info")
    return
  }
  if (action === "disable") {
    if (args.length > 0 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync disable", "error")
      return
    }
    const result = await applyWorkspaceLiveSyncModeChange(deps, "unrestricted")
    deps.flashFooter(formatWorkspaceLiveSyncModeChangeMessage("unrestricted", {
      providerReload: workspaceLiveSyncProviderReloadSummary(result),
    }), "info")
    return
  }
  if (action === "mode") {
    const mode = parseWorkspaceLiveSyncModeCommand(args[0] ?? "")
    if (!mode || args.length !== 1 || !deps.setWorkspaceLiveSyncMode) {
      deps.flashFooter("usage: /workspace sync mode off|managed|tracked", "error")
      return
    }
    const result = await applyWorkspaceLiveSyncModeChange(deps, workspaceLiveSyncModeProtocolValue(mode))
    deps.flashFooter(formatWorkspaceLiveSyncModeChangeMessage(mode, {
      providerReload: workspaceLiveSyncProviderReloadSummary(result),
    }), "info")
    return
  }
  if (action === "link") {
    await attachWorkspaceLink(deps, args[0], args[1], {
      usage: "/workspace sync link <name-or-id> [repo-root]",
      success: (repoRoot, link) => `linked ${repoRoot} for workspace live sync via ${link.name}; next: /workspace sync managed, or /workspace sync tracked on workers without managed write fencing`,
    })
    return
  }
  deps.flashFooter("usage: /workspace sync status|doctor|targets|conflicts|ignore|audit|off|managed|tracked|default|link", "error")
}

async function applyWorkspaceLiveSyncModeChange(
  deps: WorkspaceCommandHandlerDeps,
  mode: WorkspaceLiveSyncModeProtocolValue,
): Promise<unknown> {
  if (!deps.setWorkspaceLiveSyncMode) return null
  const result = await deps.setWorkspaceLiveSyncMode(deps.sessionState().id, mode)
  const session = workspaceLiveSyncModeUpdateSession(result)
  if (session) {
    deps.applySessionState(session)
  }
  if (deps.getWorkspaceLiveSyncStatus) {
    const status = await deps.getWorkspaceLiveSyncStatus()
    deps.setWorkspaceLiveSyncStatus?.(status)
  }
  return result
}

function workspaceLiveSyncModeUpdateSession(result: unknown): RuntimeSession | null {
  if (!result || typeof result !== "object" || !("session" in result)) return null
  const session = (result as { session?: unknown }).session
  return session && typeof session === "object" && "id" in session
    ? session as RuntimeSession
    : null
}

function workspaceLiveSyncProviderReloadSummary(result: unknown): WorkspaceLiveSyncProviderReloadSummary | null {
  if (!result || typeof result !== "object" || !("effects" in result)) return null
  const effects = (result as { effects?: unknown }).effects
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
    const runtime = ` machine=${target.machine_id || "-"} kernel=${target.kernel_id || "-"}`
    return `- ${target.status} ${target.link_name}: ${target.user_id} ${target.repo_root}${branch}${runtime}`
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

function formatWorkspaceLiveSyncDoctor(status: WorkspaceLiveSyncStatus): string {
  const health = workspaceLiveSyncHealthLabel(status)
  const next = workspaceLiveSyncNextAction(status)
  const lines = [
    `Workspace live sync doctor: ${health}`,
    `Mode: ${status.mode}`,
    `Footer: ${status.footer_state}`,
    "Scope: selected workspace/worktree only; other repositories are unrestricted",
    `Groups: ${status.sync_groups.length}`,
    `Targets: ${status.targets.length}`,
    `Ready targets: ${status.sync_groups.reduce((sum, group) => sum + group.ready_targets, 0)}`,
    `Degraded targets: ${status.sync_groups.reduce((sum, group) => sum + group.degraded_targets, 0)}`,
    `Conflicted targets: ${status.sync_groups.reduce((sum, group) => sum + group.conflicted_targets, 0)}`,
    `Conflicts: ${status.conflicts.length}`,
  ]
  const problems = workspaceLiveSyncProblems(status)
  if (problems.length > 0) {
    lines.push("Problems:")
    lines.push(...problems.map((problem) => `- ${problem}`))
  } else {
    lines.push("Problems: none")
  }
  if (next) lines.push(`Next: ${next}`)
  lines.push("Inspect: /workspace sync targets; /workspace sync conflicts; /workspace sync ignore; /workspace sync audit")
  return lines.join("\n")
}

export function formatWorkspaceLiveSyncAudit(events: readonly RecallEvent[]): string {
  if (events.length === 0) {
    return "No workspace live sync audit events.\nNext: change mode with /workspace sync off|managed|tracked, then rerun /workspace sync audit"
  }
  return [
    `Workspace live sync audit: ${events.length}`,
    ...events.map(formatWorkspaceLiveSyncAuditEvent),
    "Next: use /workspace sync status for current health, /workspace sync conflicts for unresolved fanout conflicts",
  ].join("\n")
}

function formatWorkspaceLiveSyncAuditEvent(event: RecallEvent): string {
  const metadata = event.metadata ?? {}
  const timestamp = Number.isFinite(event.timestamp_ms)
    ? new Date(event.timestamp_ms).toISOString()
    : "unknown-time"
  const previousMode = metadataString(metadata.previous_mode) ?? "config-default"
  const mode = metadataString(metadata.mode) ?? "unknown"
  const caller = metadataString(metadata.caller_user_id) ?? "unknown-user"
  const source = metadataString(metadata.command_source) ?? "unknown-source"
  const callerKind = metadataString(metadata.caller_kind)
  const client = metadataString(metadata.client_id)
  const machine = metadataString(metadata.machine_id)
  const scope = metadataString(metadata.scope) ?? "selected_workspace_worktree"
  const otherRepos = metadataString(metadata.other_repositories) ?? "unrestricted"
  return [
    `- ${timestamp} ${previousMode} -> ${mode} by ${caller} via ${source}`,
    `  scope=${scope}; other_repositories=${otherRepos}`,
    [
      callerKind ? `caller=${callerKind}` : null,
      client ? `client=${client}` : null,
      machine ? `machine=${machine}` : null,
      event.worktree_path ? `worktree=${event.worktree_path}` : null,
    ].filter(Boolean).join(" "),
  ].filter(Boolean).join("\n")
}

function metadataString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null
}

function readNumberOption(args: string[], flag: string): number | null {
  const index = args.indexOf(flag)
  if (index === -1) return null
  const value = args[index + 1]
  if (!value || value.startsWith("--")) return null
  const parsed = Number.parseInt(value, 10)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

function workspaceLiveSyncNextAction(status: WorkspaceLiveSyncStatus): string {
  if (status.conflicts.length > 0 || status.footer_state === "conflict") {
    return "inspect /workspace sync conflicts, ask an agent to reconcile, then rerun /workspace sync status"
  }
  if (status.mode === "unrestricted" || status.footer_state === "off") {
    return "enable with /workspace sync tracked for turn-end fanout, or use /workspace sync managed on hosts with managed write fencing"
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

function workspaceLiveSyncHealthLabel(status: WorkspaceLiveSyncStatus): string {
  if (status.conflicts.length > 0 || status.footer_state === "conflict") return "conflict"
  if (status.sync_groups.some((group) => group.degraded_targets > 0) || status.targets.some((target) => target.status === "degraded")) return "degraded"
  if (status.mode === "unrestricted" || status.footer_state === "off") return "off"
  if (status.targets.length === 0) return "no-targets"
  if (status.footer_state === "syncing") return "syncing"
  return "healthy"
}

function workspaceLiveSyncProblems(status: WorkspaceLiveSyncStatus): string[] {
  const problems: string[] = []
  if (status.mode === "unrestricted" || status.footer_state === "off") {
    problems.push("live sync is off for this session")
  }
  if (status.targets.length === 0 && status.mode !== "unrestricted") {
    problems.push("no synced worktrees or remote attachments are linked")
  }
  for (const group of status.sync_groups) {
    if (group.degraded_targets > 0) {
      problems.push(`${group.group_name} has ${group.degraded_targets} degraded target${group.degraded_targets === 1 ? "" : "s"}`)
    }
    if (group.conflicted_targets > 0) {
      problems.push(`${group.group_name} has ${group.conflicted_targets} conflicted target${group.conflicted_targets === 1 ? "" : "s"}`)
    }
  }
  for (const conflict of status.conflicts) {
    problems.push(`${conflict.path} blocked on ${conflict.target_user_id}:${conflict.target_repo_root}`)
  }
  return problems
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
      ?? `attached ${repoRoot} to workspace link ${payload.link.name}; enable with /workspace sync tracked, or use /workspace sync managed on hosts with managed write fencing`,
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
