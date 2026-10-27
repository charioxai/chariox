import type {
  RuntimeSession,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import type { RecallEvent } from "@chariox/kernel-client"
import type { LocalIpcClient } from "./ipc.js"
import {
  attachWorkspaceLinkRequest,
  createWorkspaceLinkRequest,
  detachWorkspaceLinkRequest,
  getWorkspaceLiveSyncStatusRequest,
  listWorkspaceLinksRequest,
  queryRecallRequest,
  showWorkspaceLinkRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export type WorkspaceLinkPayload = {
  link: WorkspaceLinkDefinition
  session?: RuntimeSession
}

export async function createWorkspaceLink(
  client: LocalIpcClient,
  sessionId: string,
  name: string,
): Promise<WorkspaceLinkPayload> {
  const response = await client.send<Record<string, unknown>>(createWorkspaceLinkRequest(sessionId, name))
  return expectVariant<WorkspaceLinkPayload>(response, "WorkspaceLinkCreated")
}

export async function listWorkspaceLinks(
  client: LocalIpcClient,
  sessionId: string,
): Promise<WorkspaceLinkDefinition[]> {
  const response = await client.send<Record<string, unknown>>(listWorkspaceLinksRequest(sessionId))
  return expectVariant<{ links: WorkspaceLinkDefinition[] }>(response, "WorkspaceLinksListed").links
}

export async function showWorkspaceLink(
  client: LocalIpcClient,
  sessionId: string,
  linkRef: string,
): Promise<WorkspaceLinkDefinition> {
  const response = await client.send<Record<string, unknown>>(showWorkspaceLinkRequest(sessionId, linkRef))
  return expectVariant<{ link: WorkspaceLinkDefinition }>(response, "WorkspaceLinkShown").link
}

export async function attachWorkspaceLink(
  client: LocalIpcClient,
  sessionId: string,
  linkRef: string,
  repoRoot?: string | null,
): Promise<WorkspaceLinkPayload> {
  const response = await client.send<Record<string, unknown>>(
    attachWorkspaceLinkRequest(sessionId, linkRef, repoRoot ?? null),
  )
  return expectVariant<WorkspaceLinkPayload>(response, "WorkspaceLinkAttached")
}

export async function detachWorkspaceLink(
  client: LocalIpcClient,
  sessionId: string,
  linkRef: string,
  repoRoot?: string | null,
): Promise<WorkspaceLinkPayload & { detached: unknown[] }> {
  const response = await client.send<Record<string, unknown>>(
    detachWorkspaceLinkRequest(sessionId, linkRef, repoRoot ?? null),
  )
  return expectVariant<WorkspaceLinkPayload & { detached: unknown[] }>(response, "WorkspaceLinkDetached")
}

export async function getWorkspaceLiveSyncStatus(
  client: LocalIpcClient,
  sessionId: string,
): Promise<WorkspaceLiveSyncStatus> {
  const response = await client.send<Record<string, unknown>>(getWorkspaceLiveSyncStatusRequest(sessionId))
  return expectVariant<{ status: WorkspaceLiveSyncStatus }>(response, "WorkspaceLiveSyncStatus").status
}

export async function listWorkspaceLiveSyncAudit(
  client: LocalIpcClient,
  sessionId: string,
  limit?: number | null,
): Promise<RecallEvent[]> {
  const response = await client.send<Record<string, unknown>>(queryRecallRequest({
    session_id: sessionId,
    kind: "workspace_live_sync_mode_changed",
    limit: limit ?? 20,
  }))
  return expectVariant<{ events: RecallEvent[] }>(response, "RecallEvents").events
}
