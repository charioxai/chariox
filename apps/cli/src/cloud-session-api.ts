import type { CollaborationLevel, RuntimeSession } from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  createSessionInviteRequest,
  joinSessionInviteRequest,
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
  collaborationLevel: CollaborationLevel = "private",
): Promise<LocalSessionInviteCreated> {
  const response = await client.send<Record<string, unknown>>(
    createSessionInviteRequest(sessionId, expiresInMs, maxUses, collaborationLevel),
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
