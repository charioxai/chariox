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

export type SessionHistoryOutlineWithBlobs = SessionHistoryOutline & {
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
    getSessionHistoryOutlineRequest(sessionId, agentIds, latestPromptCount, cursor),
  )
  const outline = expectVariant<SessionHistoryOutline>(response, "SessionHistoryOutline")
  return {
    ...outline,
    agents: await Promise.all(outline.agents.map(async (agent) => ({
      ...agent,
      loadedBlobs: await getSessionHistoryBlobContents(client, sessionId, agent.agent_id, sessionHistoryOutlinePrefetchBlobIds(agent)),
    }))),
  }
}

// Protocol ≥508 batch; an older kernel leaves the rows as placeholders that load on click.
async function getSessionHistoryBlobContents(
  client: LocalIpcClient,
  sessionId: string,
  agentId: string,
  blobIds: readonly string[],
): Promise<SessionHistoryBlobContent[]> {
  if (blobIds.length === 0) {
    return []
  }
  try {
    const response = await client.send<Record<string, unknown>>(
      getSessionHistoryBlobContentsRequest(sessionId, agentId, blobIds),
    )
    return expectVariant<{ blobs: SessionHistoryBlobContent[] }>(response, "SessionHistoryBlobContents").blobs
  } catch {
    return []
  }
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
