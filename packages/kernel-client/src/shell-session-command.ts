import type {
  RuntimeSession,
  SessionConfigState,
  SessionInvite,
  SessionMember,
} from "./kernel-types.js"
import {
  createSessionInviteRequest,
  createSessionRequest,
  getSessionStateRequest,
  joinSessionInviteRequest,
  listSessionMembersRequest,
  listSessionsRequest,
  resolveSessionRequest,
  revokeSessionInviteRequest,
  updateSessionConfigRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import {
  parseExecutionMode,
  parsePermissionLevel,
} from "./shell-agent-policy.js"
import {
  parsePlacementOptions,
  resolveShellPlacement,
  type ShellPlacementDeps,
} from "./shell-placement.js"
import {
  formatSessionInvite,
  formatSessionList,
  formatSessionMembers,
} from "./shell-session-format.js"
import {
  attachShellSession,
  resolveShellAttachmentId,
} from "./shell-session-attachment.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellSessionCommandDeps = ShellPlacementDeps & {
  client: ShellKernelClient
  clientId?: string | undefined
}

export async function executeSessionCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellSessionCommandDeps,
): Promise<ShellCommandResult> {
  const [action, ...args] = parsed.args
  switch (action) {
    case "list":
    case "ls": {
      const response = await deps.client.send(listSessionsRequest())
      const sessions = expectVariant<{ sessions: RuntimeSession[] }>(response, "SessionsListed").sessions
      return {
        ok: true,
        message: formatSessionList(sessions, context.sessionId),
        data: { sessions },
      }
    }
    case "new":
    case "create": {
      const placement = parsePlacementOptions(args, false)
      if (placement.error) {
        return { ok: false, message: placement.error }
      }
      if (placement.options.positional.length > 1) {
        return { ok: false, message: "usage: session new [directory] [--dir <directory>] [--worktree <directory> --branch <branch>]" }
      }
      const worktree = (await resolveShellPlacement(placement.options, context.worktree, "session working directory", deps))
        ?? context.worktree
      const response = await deps.client.send(createSessionRequest(context.workspace, worktree))
      const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionCreated")
      const session = payload.session
      const attachmentId = await attachShellSession(session.id, deps)
      const contextUpdates = {
        sessionId: session.id,
        ...(attachmentId ? { attachmentId } : {}),
        agentId: session.focused_agent_id ?? undefined,
        workspace: session.workspace_id,
        worktree: session.worktree_id,
      }
      return resourceResult(
        `created session ${session.alias ?? session.id} in ${session.worktree_id}`,
        parsed.assignment,
        session.id,
        contextUpdates,
        { session },
      )
    }
    case "attach":
    case "use": {
      const sessionRef = args[0]
      if (!sessionRef) {
        return { ok: false, message: `usage: session ${action} <ref>` }
      }
      const response = await deps.client.send(resolveSessionRequest(sessionRef, context.workspace))
      const session = expectVariant<{ session: RuntimeSession }>(response, "SessionResolved").session
      const attachmentId = await attachShellSession(session.id, deps)
      const contextUpdates = {
        sessionId: session.id,
        ...(attachmentId ? { attachmentId } : context.attachmentId ? { attachmentId: context.attachmentId } : {}),
        agentId: session.focused_agent_id ?? undefined,
        workspace: session.workspace_id,
        worktree: session.worktree_id,
      }
      return resourceResult(
        `current session = ${session.alias ?? session.id}`,
        parsed.assignment,
        session.id,
        contextUpdates,
        { session },
      )
    }
    case "mode": {
      if (!context.sessionId) {
        return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
      }
      const nextMode = parseExecutionMode(args[0])
      if (!args[0]) {
        const response = await deps.client.send(getSessionStateRequest(context.sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        return {
          ok: true,
          message: `session mode = ${parseExecutionMode(session.config_state?.values?.["agents.mode"]) ?? "build"}`,
          data: { session },
        }
      }
      if (!nextMode) {
        return { ok: false, message: "usage: session mode <build|plan>" }
      }
      const attachmentId = await resolveShellAttachmentId(context, deps)
      if (!attachmentId.ok) {
        return { ok: false, message: attachmentId.message }
      }
      const response = await deps.client.send(
        updateSessionConfigRequest(context.sessionId, attachmentId.attachmentId, { "agents.mode": nextMode }, false),
      )
      const payload = expectVariant<{ session: RuntimeSession; config: SessionConfigState }>(response, "SessionConfigUpdated")
      return {
        ok: true,
        message: `session mode = ${nextMode}`,
        data: payload,
      }
    }
    case "permissions": {
      if (!context.sessionId) {
        return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
      }
      const nextLevel = parsePermissionLevel(args[0])
      if (!args[0]) {
        const response = await deps.client.send(getSessionStateRequest(context.sessionId))
        const session = expectVariant<{ session: RuntimeSession }>(response, "SessionState").session
        return {
          ok: true,
          message: `session permissions = ${parsePermissionLevel(session.config_state?.values?.["agents.permissions"]) ?? "yolo"}`,
          data: { session },
        }
      }
      if (!nextLevel) {
        return { ok: false, message: "usage: session permissions <required|yolo>" }
      }
      const attachmentId = await resolveShellAttachmentId(context, deps)
      if (!attachmentId.ok) {
        return { ok: false, message: attachmentId.message }
      }
      const response = await deps.client.send(
        updateSessionConfigRequest(context.sessionId, attachmentId.attachmentId, { "agents.permissions": nextLevel }, false),
      )
      const payload = expectVariant<{ session: RuntimeSession; config: SessionConfigState }>(response, "SessionConfigUpdated")
      return {
        ok: true,
        message: `session permissions = ${nextLevel}`,
        data: payload,
      }
    }
    case "members": {
      const sessionId = args[0] ?? context.sessionId
      if (!sessionId) {
        return { ok: false, message: "usage: session members [session-ref]" }
      }
      const response = await deps.client.send(listSessionMembersRequest(sessionId))
      const payload = expectVariant<{ members: SessionMember[]; invites: SessionInvite[] }>(response, "SessionMembersListed")
      return { ok: true, message: formatSessionMembers(payload.members, payload.invites), data: payload }
    }
    case "invite": {
      const [inviteAction, maxUsesRaw] = args
      if (inviteAction !== "create") {
        return { ok: false, message: "usage: session invite create [max-uses]" }
      }
      if (!context.sessionId) {
        return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
      }
      const maxUses = maxUsesRaw ? Number.parseInt(maxUsesRaw, 10) : 1
      if (!Number.isFinite(maxUses) || maxUses <= 0) {
        return { ok: false, message: "usage: session invite create [max-uses]" }
      }
      const response = await deps.client.send(createSessionInviteRequest(context.sessionId, null, maxUses))
      const payload = expectVariant<{ invite: { invite: SessionInvite; invite_token: string }; session: RuntimeSession }>(response, "SessionInviteCreated")
      return {
        ok: true,
        message: formatSessionInvite(payload.invite.invite, payload.invite.invite_token),
        data: payload,
        contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    case "join": {
      const [inviteToken, userId] = args
      if (!inviteToken || !userId) {
        return { ok: false, message: "usage: session join <invite-token> <user-id>" }
      }
      const response = await deps.client.send(joinSessionInviteRequest(inviteToken, userId))
      const payload = expectVariant<{ member: SessionMember; session: RuntimeSession }>(response, "SessionInviteJoined")
      const attachmentId = await attachShellSession(payload.session.id, deps)
      return {
        ok: true,
        message: `joined session ${payload.session.alias ?? payload.session.id} as ${payload.member.user_id}`,
        data: payload,
        contextUpdates: {
          sessionId: payload.session.id,
          ...(attachmentId ? { attachmentId } : {}),
          agentId: payload.session.focused_agent_id ?? undefined,
          workspace: payload.session.workspace_id,
          worktree: payload.session.worktree_id,
        },
      }
    }
    case "revoke-invite": {
      const inviteRef = args[0]
      if (!context.sessionId || !inviteRef) {
        return { ok: false, message: "usage: session revoke-invite <invite-id>" }
      }
      const response = await deps.client.send(revokeSessionInviteRequest(context.sessionId, inviteRef))
      const payload = expectVariant<{ invite: SessionInvite; session: RuntimeSession }>(response, "SessionInviteRevoked")
      return {
        ok: true,
        message: `revoked session invite ${payload.invite.invite_id}`,
        data: payload,
        contextUpdates: { sessionId: payload.session.id, agentId: payload.session.focused_agent_id ?? undefined },
      }
    }
    default:
      return { ok: false, message: "usage: session list|new|attach|use|members|invite|join|revoke-invite|mode|permissions" }
  }
}

function resourceResult(
  message: string,
  assignment: string | undefined,
  value: string,
  contextUpdates: ShellCommandResult["contextUpdates"],
  data: unknown,
): ShellCommandResult {
  return {
    ok: true,
    message,
    data,
    bindings: assignment ? { [assignment]: value } : undefined,
    contextUpdates,
  }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
