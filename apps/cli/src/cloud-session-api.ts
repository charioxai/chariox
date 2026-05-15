import type { RuntimeSession } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  acceptCloudSessionInviteRequest,
  createCloudSessionInviteRequest,
  createSessionInviteRequest,
  joinSessionInviteRequest,
  listCloudCollaboratorsRequest,
  listCloudSessionMembersRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export type LocalSessionInviteCreated = {
  invite: { invite_token: string; invite: { invite_id: string } }
  session: RuntimeSession
}

export type LocalSessionInviteJoined = {
  member: { user_id: string }
  session: RuntimeSession
}

export async function createSessionInvite(
  client: LocalIpcClient,
  sessionId: string,
  expiresInMs: number | null,
  maxUses: number | null,
): Promise<LocalSessionInviteCreated> {
  const response = await client.send<Record<string, unknown>>(
    createSessionInviteRequest(sessionId, expiresInMs, maxUses),
  )
  return expectVariant<LocalSessionInviteCreated>(response, "SessionInviteCreated")
}

export async function joinSessionInvite(
  client: LocalIpcClient,
  inviteToken: string,
  userId: string,
): Promise<LocalSessionInviteJoined> {
  const response = await client.send<Record<string, unknown>>(joinSessionInviteRequest(inviteToken, userId))
  return expectVariant<LocalSessionInviteJoined>(response, "SessionInviteJoined")
}

export async function createCloudSessionInvite(
  client: LocalIpcClient,
  sessionId: string,
  inviteOptions: { displayName?: string | null; expiresInMs?: number | null; maxUses?: number | null },
): Promise<Record<string, unknown>> {
  const response = await client.send<Record<string, unknown>>(
    createCloudSessionInviteRequest(sessionId, inviteOptions),
  )
  return expectVariant<Record<string, unknown>>(response, "CloudSessionInviteCreated")
}

export async function acceptCloudSessionInvite(
  client: LocalIpcClient,
  inviteToken: string,
): Promise<Record<string, unknown>> {
  const response = await client.send<Record<string, unknown>>(acceptCloudSessionInviteRequest(inviteToken))
  return expectVariant<Record<string, unknown>>(response, "CloudSessionInviteAccepted")
}

export async function listCloudSessionMembers(
  client: LocalIpcClient,
  sessionId: string,
): Promise<Record<string, unknown>> {
  const response = await client.send<Record<string, unknown>>(listCloudSessionMembersRequest(sessionId))
  return expectVariant<Record<string, unknown>>(response, "CloudSessionMembersListed")
}

export async function listCloudCollaborators(client: LocalIpcClient): Promise<Record<string, unknown>[]> {
  const response = await client.send<Record<string, unknown>>(listCloudCollaboratorsRequest())
  return expectVariant<{ collaborators: Record<string, unknown>[] }>(
    response,
    "CloudCollaboratorsListed",
  ).collaborators
}
