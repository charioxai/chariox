import type {
  PromptInputHistoryEntry,
  PromptInputHistoryPage,
  SessionHistoryBlobContent,
  SessionHistoryOutlineCursor,
  SessionHistoryOutline,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getPromptInputHistoryRequest,
  recordPromptInputHistoryRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function getSessionHistoryOutline(
  client: LocalIpcClient,
  sessionId: string,
  agentIds: readonly string[],
  latestPromptCount = 4,
  cursor: SessionHistoryOutlineCursor | null = null,
): Promise<SessionHistoryOutline> {
  const response = await client.send<Record<string, unknown>>(
    getSessionHistoryOutlineRequest(sessionId, agentIds, latestPromptCount, cursor),
  )
  return expectVariant<SessionHistoryOutline>(response, "SessionHistoryOutline")
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
