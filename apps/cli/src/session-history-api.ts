import type {
  PromptInputHistoryEntry,
  PromptInputHistoryPage,
  SessionHistoryCursor,
  SessionHistoryPage,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import {
  getPromptInputHistoryRequest,
  getSessionHistoryRequest,
  recordPromptInputHistoryRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

const BOOTSTRAP_HISTORY_MAX_CHARS = 100_000
const HISTORY_PAGE_ROUND_COUNT = 1

export async function getSessionHistory(
  client: LocalIpcClient,
  sessionId: string,
  cursor?: SessionHistoryCursor | null,
  agentId?: string | null,
): Promise<SessionHistoryPage> {
  const response = await client.send<Record<string, unknown>>(
    getSessionHistoryRequest(sessionId, HISTORY_PAGE_ROUND_COUNT, BOOTSTRAP_HISTORY_MAX_CHARS, cursor, agentId),
  )
  return expectVariant<SessionHistoryPage>(response, "SessionHistory")
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
