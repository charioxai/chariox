import type {
  RuntimeSession,
  WorkspaceLinkDefinition,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  attachWorkspaceLinkRequest,
  createWorkspaceLinkRequest,
  detachWorkspaceLinkRequest,
  listWorkspaceLinksRequest,
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
