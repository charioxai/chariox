import {
  normalizeRuntimeSession,
  normalizeRuntimeSessions,
  type RuntimeAttachment,
  type RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  aliasSessionRequest,
  attachToSessionRequest,
  createSessionRequest,
  deleteSessionRequest,
  detachFromSessionRequest,
  endSessionRequest,
  getSessionStateRequest,
  listSessionsRequest,
  resolveSessionRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import { resolvePendingWaitingRoomWorktreePath } from "./waiting-room-worktrees.js"

export async function listSessions(client: LocalIpcClient): Promise<RuntimeSession[]> {
  const response = await client.send<Record<string, unknown>>(listSessionsRequest())
  const payload = expectVariant<{ sessions: RuntimeSession[] }>(response, "SessionsListed")
  return normalizeRuntimeSessions(payload.sessions).sort((left, right) => right.created_at_ms - left.created_at_ms)
}

export async function createSession(
  client: LocalIpcClient,
  workspace: string,
  worktree: string,
  alias?: string,
  agentDefaults?: RuntimeSession["agent_defaults"],
  sliceRef?: string | null,
): Promise<RuntimeSession> {
  const resolvedWorktree = await resolvePendingWaitingRoomWorktreePath(workspace, worktree)
  const response = await client.send<Record<string, unknown>>(
    createSessionRequest(workspace, resolvedWorktree, alias, agentDefaults, sliceRef),
  )
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionCreated")
  return normalizeRuntimeSession(payload.session)
}

export async function resolveSession(
  client: LocalIpcClient,
  sessionRef: string,
  workspace: string,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(resolveSessionRequest(sessionRef, workspace))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionResolved")
  return normalizeRuntimeSession(payload.session)
}

export async function aliasSession(
  client: LocalIpcClient,
  sessionId: string,
  alias: string,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(aliasSessionRequest(sessionId, alias))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionAliased")
  return normalizeRuntimeSession(payload.session)
}

export async function deleteSessionByRef(
  client: LocalIpcClient,
  sessionRef: string,
  workspace: string,
): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(deleteSessionRequest(sessionRef, workspace))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionDeleted")
  return normalizeRuntimeSession(payload.session)
}

export async function archiveSessionById(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(endSessionRequest(sessionId))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionEnded")
  return normalizeRuntimeSession(payload.session)
}

export async function detachSessionAttachment(client: LocalIpcClient, attachmentId: string): Promise<void> {
  await client.send<Record<string, unknown>>(detachFromSessionRequest(attachmentId))
}

export async function attachToSession(
  client: LocalIpcClient,
  sessionId: string,
  clientId: string,
): Promise<RuntimeAttachment> {
  const response = await client.send<Record<string, unknown>>(attachToSessionRequest(sessionId, clientId))
  const payload = expectVariant<{ attachment: RuntimeAttachment }>(response, "SessionAttached")
  return payload.attachment
}

export async function getSessionState(client: LocalIpcClient, sessionId: string): Promise<RuntimeSession> {
  const response = await client.send<Record<string, unknown>>(getSessionStateRequest(sessionId))
  const payload = expectVariant<{ session: RuntimeSession }>(response, "SessionState")
  return normalizeRuntimeSession(payload.session)
}
