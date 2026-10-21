import type {
  PromptInputHistoryEntry,
  PromptInputHistoryPage,
  SessionHistoryBlobContent,
  SessionHistoryOutlineCursor,
  SessionHistoryOutline,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { sessionHistoryOutlinePrefetchBlobIds } from "@chariox/kernel-client/session-history-outline"
import {
  getSessionHistoryBlobContentRequest,
  getSessionHistoryBlobContentsRequest,
  getSessionHistoryOutlineRequest,
  getPromptInputHistoryRequest,
  recordPromptInputHistoryRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export type SessionHistoryOutlineWithBlobs = Omit<SessionHistoryOutline, "agents"> & {
  agents: (SessionHistoryOutline["agents"][number] & { loadedBlobs: SessionHistoryBlobContent[] })[]
}

export async function getSessionHistoryOutline(
  client: LocalIpcClient,
  sessionId: string,
  agentIds: readonly string[],
  latestPromptCount = 4,
  cursor: SessionHistoryOutlineCursor | null = null,
): Promise<SessionHistoryOutlineWithBlobs> {
  const response = await client.send<Record<string, unknown>>(
    getSessionHistoryOutlineRequest(sessionId, agentIds, latestPromptCount, cursor, true),
  )
  const outline = expectVariant<SessionHistoryOutline>(response, "SessionHistoryOutline")
  return {
    ...outline,
    agents: await Promise.all(outline.agents.map(async (agent) => ({
      ...agent,
      loadedBlobs: agent.prefetched_blobs ?? await getSessionHistoryBlobContents(client, sessionId, agent.agent_id, sessionHistoryOutlinePrefetchBlobIds(agent)),
    }))),
  }
}

// Protocol ≥508 batch; an older kernel leaves the rows as placeholders that load on click.
export async function getSessionHistoryBlobContents(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  blobIds: readonly string[],
  fallback = false,
): Promise<SessionHistoryBlobContent[]> {
  if (blobIds.length === 0) {
    return []
  }
  const contents: SessionHistoryBlobContent[] = []
  for (let offset = 0; offset < blobIds.length; offset += 64) {
    const ids = blobIds.slice(offset, offset + 64)
    try {
      const response = await client.send<Record<string, unknown>>(getSessionHistoryBlobContentsRequest(sessionId, agentId, ids))
      contents.push(...expectVariant<{ blobs: SessionHistoryBlobContent[] }>(response, "SessionHistoryBlobContents").blobs)
    } catch {
      if (!fallback) continue
      // A pre-508 kernel rejects the batch. Preserve its sequential load path.
      for (const id of ids) contents.push(await getSessionHistoryBlobContent(client, sessionId, agentId, id))
    }
  }
  return contents
}

export async function getSessionHistoryBlobContent(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  blobId: string,
): Promise<SessionHistoryBlobContent> {
  const response = await client.send<Record<string, unknown>>(
    getSessionHistoryBlobContentRequest(sessionId, agentId, blobId),
  )
  return expectVariant<SessionHistoryBlobContent>(response, "SessionHistoryBlobContent")
}

export async function getPromptInputHistory(
  client: LocalIpcClient,
  sessionId: string,
  afterSequence: number | null = null,
  limit = 5000,
): Promise<PromptInputHistoryPage> {
  const response = await client.send<Record<string, unknown>>(
    getPromptInputHistoryRequest(sessionId, afterSequence, limit),
  )
  return expectVariant<PromptInputHistoryPage>(response, "PromptInputHistory")
}

export async function recordPromptInputHistory(
  client: LocalIpcClient,
  sessionId: string,
  attachmentId: string | null,
  kind: PromptInputHistoryEntry["kind"],
  text: string,
): Promise<PromptInputHistoryEntry> {
  const response = await client.send<Record<string, unknown>>(
    recordPromptInputHistoryRequest(sessionId, attachmentId, kind, text),
  )
  const payload = expectVariant<{ entry: PromptInputHistoryEntry }>(response, "PromptInputHistoryRecorded")
  return payload.entry
}
