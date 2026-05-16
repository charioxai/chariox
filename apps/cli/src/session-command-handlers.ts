import type {
  RuntimeAttachment,
  RuntimeSession,
  SessionConfigState,
} from "./cli-types.js"
import type { ParsedSlashCommand } from "./commands.js"
import {
  parsePlacementOptions,
  resolveExistingLocalDirectory,
  resolveLocalPlacement,
  type LocalGitWorktreeOptions,
} from "./command-worktree-placement.js"
import type { SessionListEntry } from "./sessions.js"

const SESSION_AGENT_MODE_CONFIG_KEY = "agents.mode"
const SESSION_AGENT_PERMISSION_CONFIG_KEY = "agents.permissions"

type FooterTone = "info" | "error"

type CreateSessionResult = Pick<RuntimeSession, "id" | "alias">
type ResolveSessionResult = Pick<RuntimeSession, "id" | "alias">
type DeleteSessionResult = Pick<RuntimeSession, "id" | "alias">

export type SessionCommandHandlerDeps = {
  currentWorkspaceTarget: () => string
  currentWorktreeTarget: () => string
  accountProfile?: string | null
  isAttached: () => boolean
  sessionState: () => RuntimeSession
  attachmentState: () => RuntimeAttachment | null
  currentModelId: () => string
  currentVariantId: () => string
  currentProviderId: () => string
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  formatError: (error: unknown) => string
  createSession: (
    workspace: string,
    worktree: string,
    alias?: string,
    agentDefaults?: RuntimeSession["agent_defaults"],
  ) => Promise<CreateSessionResult>
  prepareLocalGitWorktree?: (options: LocalGitWorktreeOptions) => Promise<string>
  attachBinding: (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
  ) => Promise<void>
  resolveSession: (reference: string, workspace: string) => Promise<ResolveSessionResult>
  listSessions: () => Promise<RuntimeSession[]>
  deleteSessionByRef: (reference: string, workspace: string) => Promise<DeleteSessionResult>
  assignSessionAlias?: (sessionId: string, alias: string) => Promise<RuntimeSession>
  transitionToNoSession: (message: string) => void
  updateSessionConfig: (
    sessionId: string,
    attachmentId: string,
    values: Record<string, string>,
    requiresIdle: boolean,
  ) => Promise<{ session: RuntimeSession; config: SessionConfigState }>
  applySessionState: (session: RuntimeSession) => void
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  formatSessionList: (sessions: SessionListEntry[], currentSessionId?: string) => string
}

export async function handleSessionSlashCommand(
  deps: SessionCommandHandlerDeps,
  command: Extract<ParsedSlashCommand, { kind: "session" }>,
): Promise<boolean> {
  const { action, value, args } = command

  switch (action) {
    case "create":
    case "new":
      return createSession(deps, args)
    case "attach":
      return attachSession(deps, value)
    case "list":
    case "ls":
      return listSessions(deps)
    case "mode":
      return setSessionMode(deps, value)
    case "permissions":
      return setSessionPermissions(deps, value)
    case "delete":
      return deleteSession(deps, value)
    default:
      return setSessionAlias(deps, action, args)
  }
}

async function createSession(
  deps: SessionCommandHandlerDeps,
  args: string[],
): Promise<boolean> {
  try {
    const parsed = parsePlacementOptions(args, "/session new", false)
    if (parsed.error) {
      deps.flashFooter(parsed.error, "error")
      return true
    }
    if (parsed.positional.length > 1) {
      deps.flashFooter("usage: /session new [directory] [--dir <directory>] [--worktree <directory> --branch <branch>]", "error")
      return true
    }
    let sessionWorktree = deps.currentWorktreeTarget()
    const positionalDirectory = parsed.positional[0]
    if (positionalDirectory && !parsed.directory && !parsed.gitWorktree && !parsed.branch && !parsed.fromRef) {
      sessionWorktree = await resolveExistingLocalDirectory(positionalDirectory, deps.currentWorktreeTarget(), "session working directory")
    } else {
      const resolvedPlacement = await resolveLocalPlacement({
        directory: parsed.directory,
        gitWorktree: parsed.gitWorktree,
        branch: parsed.branch,
        fromRef: parsed.fromRef,
        label: "session working directory",
      }, {
        baseDirectory: deps.currentWorktreeTarget(),
        prepareLocalGitWorktree: deps.prepareLocalGitWorktree,
      })
      sessionWorktree = resolvedPlacement ?? deps.currentWorktreeTarget()
    }
    const session = await deps.createSession(deps.currentWorkspaceTarget(), sessionWorktree, undefined, {
      provider: deps.currentProviderId(),
      model: deps.currentModelId(),
      effort: deps.currentVariantId(),
      account_profile: deps.accountProfile ?? null,
      execution_mode: "build",
      permission_level: "yolo",
    })
    await deps.attachBinding(session, true)
    const placement = sessionWorktree !== deps.currentWorktreeTarget() ? ` in ${sessionWorktree}` : ""
    deps.flashFooter(`attached to session ${session.alias ?? session.id}${placement}`, "info")
  } catch (error) {
    deps.flashFooter(deps.formatError(error), "error")
  }
  return true
}

async function attachSession(
  deps: SessionCommandHandlerDeps,
  value: string,
): Promise<boolean> {
  if (!value) {
    deps.flashFooter("usage: /session attach <ref>", "error")
    return true
  }
  const session = await deps.resolveSession(value, deps.currentWorkspaceTarget())
  await deps.attachBinding(session, false)
  deps.flashFooter(`attached to session ${session.alias ?? session.id}`, "info")
  return true
}

async function listSessions(deps: SessionCommandHandlerDeps): Promise<boolean> {
  const sessions = await deps.listSessions()
  deps.appendNotice(deps.formatSessionList(sessions, deps.sessionState().id))
  deps.flashFooter(`listed ${sessions.length} session${sessions.length === 1 ? "" : "s"}`, "info")
  return true
}

async function setSessionMode(
  deps: SessionCommandHandlerDeps,
  value: string,
): Promise<boolean> {
  if (!deps.attachmentState()) {
    deps.flashFooter("must be attached to change session mode", "error")
    return true
  }
  if (!value) {
    const current = parseExecutionMode(deps.sessionState().config_state?.values?.[SESSION_AGENT_MODE_CONFIG_KEY]) ?? "build"
    deps.flashFooter(`session mode: ${current}`, "info")
    return true
  }
  const nextMode = parseExecutionMode(value)
  if (!nextMode) {
    deps.flashFooter("usage: /session mode <build|plan>", "error")
    return true
  }
  const payload = await deps.updateSessionConfig(
    deps.sessionState().id,
    deps.attachmentState()!.id,
    { [SESSION_AGENT_MODE_CONFIG_KEY]: nextMode },
    false,
  )
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  deps.flashFooter(`session mode set to ${nextMode}`, "info")
  return true
}

function parseExecutionMode(value: string | null | undefined): "build" | "plan" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "build" || normalized === "plan" ? normalized : null
}

async function setSessionPermissions(
  deps: SessionCommandHandlerDeps,
  value: string,
): Promise<boolean> {
  if (!deps.attachmentState()) {
    deps.flashFooter("must be attached to change session permissions", "error")
    return true
  }
  if (!value) {
    const current = parsePermissionLevel(deps.sessionState().config_state?.values?.[SESSION_AGENT_PERMISSION_CONFIG_KEY]) ?? "yolo"
    deps.flashFooter(`session permissions: ${current}`, "info")
    return true
  }
  const nextLevel = parsePermissionLevel(value)
  if (!nextLevel) {
    deps.flashFooter("usage: /session permissions <required|yolo>", "error")
    return true
  }
  const payload = await deps.updateSessionConfig(
    deps.sessionState().id,
    deps.attachmentState()!.id,
    { [SESSION_AGENT_PERMISSION_CONFIG_KEY]: nextLevel },
    false,
  )
  deps.applySessionState(payload.session)
  await deps.refreshAgentPanes(payload.session)
  deps.flashFooter(`session permissions set to ${nextLevel}`, "info")
  return true
}

function parsePermissionLevel(value: string | null | undefined): "required" | "yolo" | null {
  if (!value) return null
  const normalized = value.trim().toLowerCase()
  return normalized === "required" || normalized === "yolo" ? normalized : null
}

async function deleteSession(
  deps: SessionCommandHandlerDeps,
  value: string,
): Promise<boolean> {
  const sessionRef = value || (deps.isAttached() ? deps.sessionState().id : "")
  if (!sessionRef) {
    deps.flashFooter("usage: /session delete <ref>", "error")
    return true
  }
  const deleted = await deps.deleteSessionByRef(sessionRef, deps.currentWorkspaceTarget())
  if (deps.isAttached() && deleted.id === deps.sessionState().id) {
    deps.transitionToNoSession(`Session ${deleted.alias ?? deleted.id} was deleted.`)
  } else {
    deps.flashFooter(`deleted session ${deleted.alias ?? deleted.id}`, "info")
  }
  return true
}

async function setSessionAlias(
  deps: SessionCommandHandlerDeps,
  action: string | null | undefined,
  args: string[],
): Promise<boolean> {
  if (!action) {
    return false
  }
  if (args.length !== 0) {
    deps.flashFooter("usage: /session <alias>", "error")
    return true
  }
  const alias = action
  if (!deps.isAttached()) {
    deps.flashFooter("attach to a session before setting an alias", "error")
    return true
  }
  if (!deps.assignSessionAlias) {
    deps.flashFooter("session aliases are unavailable in this build", "error")
    return true
  }
  const session = await deps.assignSessionAlias(deps.sessionState().id, alias)
  deps.applySessionState(session)
  deps.flashFooter(`session ${session.id} aliased as ${session.alias}`, "info")
  return true
}
