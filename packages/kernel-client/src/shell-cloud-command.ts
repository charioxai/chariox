import type {
  CloudCollaborator,
  CloudSessionMember,
  RuntimeSession,
  SessionInvite,
  SessionMember,
} from "./kernel-types.js"
import {
  acceptCloudSessionInviteRequest,
  cloudRelayStatusRequest,
  createCloudSessionInviteRequest,
  createSessionInviteRequest,
  joinSessionInviteRequest,
  listCloudCollaboratorsRequest,
  listCloudSessionMembersRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult, ShellContext } from "./shell-core.js"
import { attachShellSession } from "./shell-session-attachment.js"
import {
  formatCloudCollaborators,
  formatCloudMembers,
} from "./shell-session-format.js"

type ShellKernelClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
}

export type ShellCloudCommandDeps = {
  client: ShellKernelClient
  clientId?: string | undefined
}

export async function executeCloudCommand(
  parsed: ParsedShellCommand,
  context: ShellContext,
  deps: ShellCloudCommandDeps,
): Promise<ShellCommandResult> {
  const [area, action, ...args] = parsed.args
  if (area === "invite" && action === "create") {
    if (!context.sessionId) {
      return { ok: false, message: "no current session; run `session new` or `session use <ref>` first" }
    }
    const maxUses = args[0] ? Number.parseInt(args[0], 10) : 1
    if (!Number.isFinite(maxUses) || maxUses <= 0) {
      return { ok: false, message: "usage: cloud invite create [max-uses]" }
    }
    const localResponse = await deps.client.send(createSessionInviteRequest(context.sessionId, null, maxUses))
    const local = expectVariant<{ invite: { invite: SessionInvite; invite_token: string }; session: RuntimeSession }>(
      localResponse,
      "SessionInviteCreated",
    )
    const cloudResponse = await deps.client.send(createCloudSessionInviteRequest(context.sessionId, {
      displayName: local.session.alias ?? local.session.id,
      maxUses,
    }))
    const cloud = expectVariant<{ invite: { invite_token: string; invite_id: string } }>(
      cloudResponse,
      "CloudSessionInviteCreated",
    )
    return {
      ok: true,
      message: [
        `cloud invite ${cloud.invite.invite_id}`,
        `cloud_invite=${cloud.invite.invite_token}`,
        `local_invite=${local.invite.invite_token}`,
      ].join("\n"),
      data: { cloud, local },
      contextUpdates: { sessionId: local.session.id, agentId: local.session.focused_agent_id ?? undefined },
    }
  }
  if (area === "invite" && action === "accept") {
    const inviteToken = args[0]
    const localInviteToken = args[1]
    if (!inviteToken) {
      return { ok: false, message: "usage: cloud invite accept <cloud-invite-token> [local-invite-token]" }
    }
    const cloudResponse = await deps.client.send(acceptCloudSessionInviteRequest(inviteToken))
    const cloud = expectVariant<{ acceptance: { user_id: string } }>(cloudResponse, "CloudSessionInviteAccepted")
    if (!localInviteToken) {
      return {
        ok: true,
        message: `accepted cloud invite as ${cloud.acceptance.user_id}; provide local invite token to join the kernel session`,
        data: cloud,
      }
    }
    const joinResponse = await deps.client.send(joinSessionInviteRequest(localInviteToken, cloud.acceptance.user_id))
    const joined = expectVariant<{ member: SessionMember; session: RuntimeSession }>(joinResponse, "SessionInviteJoined")
    const attachmentId = await attachShellSession(joined.session.id, deps)
    return {
      ok: true,
      message: `joined session ${joined.session.alias ?? joined.session.id} as ${joined.member.user_id}`,
      data: { cloud, joined },
      contextUpdates: {
        sessionId: joined.session.id,
        ...(attachmentId ? { attachmentId } : {}),
        agentId: joined.session.focused_agent_id ?? undefined,
        workspace: joined.session.workspace_id,
        worktree: joined.session.worktree_id,
      },
    }
  }
  if ((area === "members" && !action) || (area === "members" && action === "list")) {
    const sessionId = context.sessionId
    if (!sessionId) {
      return { ok: false, message: "usage: cloud members [list]" }
    }
    const response = await deps.client.send(listCloudSessionMembersRequest(sessionId))
    const payload = expectVariant<{ members: CloudSessionMember[] }>(response, "CloudSessionMembersListed")
    return { ok: true, message: formatCloudMembers(payload.members), data: payload }
  }
  if ((area === "collaborators" && !action) || (area === "collaborators" && action === "list")) {
    const response = await deps.client.send(listCloudCollaboratorsRequest())
    const payload = expectVariant<{ collaborators: CloudCollaborator[] }>(response, "CloudCollaboratorsListed")
    return { ok: true, message: formatCloudCollaborators(payload.collaborators), data: payload }
  }
  if (!area || area === "status") {
    const response = await deps.client.send(cloudRelayStatusRequest())
    const payload = expectVariant<{ profile: { account_slug?: string; email?: string } | null }>(response, "CloudRelayStatus")
    return {
      ok: true,
      message: payload.profile ? `cloud profile ${payload.profile.account_slug ?? payload.profile.email ?? "configured"}` : "cloud profile not configured",
      data: payload,
    }
  }
  return { ok: false, message: "usage: cloud invite create|accept | cloud members | cloud collaborators | cloud status" }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
